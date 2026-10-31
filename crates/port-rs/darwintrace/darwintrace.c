/*
 * darwintrace for port-rs trace mode: record every path inside the MacPorts
 * prefix that a traced build opens, inspects or executes, so the files trace
 * mode hid can be reported after each target. Like upstream's darwintrace,
 * programs protected by System Integrity Protection run as patched, ad-hoc
 * signed copies, because SIP strips DYLD_INSERT_LIBRARIES from them.
 *
 * Environment: DARWINTRACE_LOG (file to append paths to), DARWINTRACE_PREFIX
 * (the prefix to watch), DARWINTRACE_SIPDIR (where SIP copies live).
 */
#include <copyfile.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <mach-o/fat.h>
#include <mach-o/loader.h>
#include <pthread.h>
#include <spawn.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/attr.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

static const char *log_path, *prefix, *sipdir, *self_path;
static size_t prefix_length;
static int log_fd = -1;
static pthread_once_t once = PTHREAD_ONCE_INIT;

static void setup(void) {
    log_path = getenv("DARWINTRACE_LOG");
    prefix = getenv("DARWINTRACE_PREFIX");
    sipdir = getenv("DARWINTRACE_SIPDIR");
    self_path = getenv("DYLD_INSERT_LIBRARIES");
    if (prefix) {
        prefix_length = strlen(prefix);
        while (prefix_length > 1 && prefix[prefix_length - 1] == '/') {
            prefix_length--;
        }
    }
    if (log_path) {
        log_fd = open(log_path, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC, 0644);
    }
}

/* Lexically resolve "." and ".." in an absolute path. */
static void normalize(char *path) {
    char *out = path, *in = path;
    while (*in) {
        while (*in == '/') {
            in++;
        }
        char *end = strchr(in, '/');
        size_t length = end ? (size_t)(end - in) : strlen(in);
        if (length == 0) {
            break;
        }
        if (length == 1 && in[0] == '.') {
            /* skip */
        } else if (length == 2 && in[0] == '.' && in[1] == '.') {
            while (out > path && *--out != '/') {
            }
        } else {
            *out++ = '/';
            memmove(out, in, length);
            out += length;
        }
        in += length;
    }
    if (out == path) {
        *out++ = '/';
    }
    *out = '\0';
}

static void record(int directory, const char *path) {
    pthread_once(&once, setup);
    if (log_fd < 0 || !path || !*path || !prefix_length) {
        return;
    }
    char full[PATH_MAX * 2];
    if (path[0] == '/') {
        if (strlen(path) >= sizeof full) {
            return;
        }
        strcpy(full, path);
    } else {
        char base[PATH_MAX];
        if (directory == AT_FDCWD) {
            if (!getcwd(base, sizeof base)) {
                return;
            }
        } else if (fcntl(directory, F_GETPATH, base) == -1) {
            return;
        }
        if ((size_t)snprintf(full, sizeof full, "%s/%s", base, path) >= sizeof full) {
            return;
        }
    }
    normalize(full);
    if (strncmp(full, prefix, prefix_length) != 0 || full[prefix_length] != '/') {
        return;
    }
    size_t length = strlen(full);
    full[length] = '\n';
    int saved = errno;
    (void)write(log_fd, full, length + 1);
    errno = saved;
}

static int sip_protected(const char *path) {
    static const char *roots[] = {"/bin/", "/sbin/", "/usr/bin/", "/usr/sbin/", "/usr/libexec/",
                                  "/System/", NULL};
    for (const char **root = roots; *root; root++) {
        if (strncmp(path, *root, strlen(*root)) == 0) {
            return 1;
        }
    }
    return 0;
}

/* arm64e slices cannot carry an ad-hoc signature; mark them plain arm64. */
static void patch_arm64e(int fd) {
    uint32_t magic;
    if (pread(fd, &magic, sizeof magic, 0) != sizeof magic) {
        return;
    }
    if (magic == FAT_CIGAM) {
        uint32_t count;
        if (pread(fd, &count, sizeof count, 4) != sizeof count) {
            return;
        }
        count = OSSwapBigToHostInt32(count);
        for (uint32_t index = 0; index < count && index < 64; index++) {
            struct fat_arch arch;
            off_t at = sizeof(struct fat_header) + index * sizeof arch;
            if (pread(fd, &arch, sizeof arch, at) != sizeof arch) {
                return;
            }
            if ((cpu_type_t)OSSwapBigToHostInt32((uint32_t)arch.cputype) == CPU_TYPE_ARM64 &&
                (OSSwapBigToHostInt32((uint32_t)arch.cpusubtype) & ~CPU_SUBTYPE_MASK) ==
                    CPU_SUBTYPE_ARM64E) {
                uint32_t plain = OSSwapHostToBigInt32(CPU_SUBTYPE_ARM64_ALL);
                (void)pwrite(fd, &plain, sizeof plain, at + offsetof(struct fat_arch, cpusubtype));
                uint32_t thin = CPU_SUBTYPE_ARM64_ALL;
                (void)pwrite(fd, &thin, sizeof thin,
                             OSSwapBigToHostInt32(arch.offset) +
                                 offsetof(struct mach_header_64, cpusubtype));
            }
        }
    } else if (magic == MH_MAGIC_64) {
        struct mach_header_64 header;
        if (pread(fd, &header, sizeof header, 0) == sizeof header &&
            header.cputype == CPU_TYPE_ARM64 &&
            (header.cpusubtype & ~CPU_SUBTYPE_MASK) == CPU_SUBTYPE_ARM64E) {
            uint32_t thin = CPU_SUBTYPE_ARM64_ALL;
            (void)pwrite(fd, &thin, sizeof thin, offsetof(struct mach_header_64, cpusubtype));
        }
    }
}

static int make_directories(char *path) {
    for (char *slash = strchr(path + 1, '/'); slash; slash = strchr(slash + 1, '/')) {
        *slash = '\0';
        int failed = mkdir(path, 0755) == -1 && errno != EEXIST;
        *slash = '/';
        if (failed) {
            return -1;
        }
    }
    return 0;
}

/* The traceable copy of a SIP-protected program, created on first use. */
static const char *sip_copy(const char *source, char *copy, size_t size) {
    pthread_once(&once, setup);
    if (!sipdir || (size_t)snprintf(copy, size, "%s%s", sipdir, source) >= size) {
        return NULL;
    }
    struct stat original, existing;
    if (stat(source, &original) == -1 || !S_ISREG(original.st_mode)) {
        return NULL;
    }
    if (stat(copy, &existing) == 0 && existing.st_mtimespec.tv_sec == original.st_mtimespec.tv_sec &&
        existing.st_size == original.st_size) {
        return copy;
    }
    if (make_directories(copy) == -1) {
        return NULL;
    }
    char staged[PATH_MAX];
    if ((size_t)snprintf(staged, sizeof staged, "%s.XXXXXX", copy) >= sizeof staged) {
        return NULL;
    }
    int fd = mkstemp(staged);
    if (fd == -1) {
        return NULL;
    }
    close(fd);
    if (copyfile(source, staged, NULL, COPYFILE_DATA) == -1) {
        unlink(staged);
        return NULL;
    }
    fd = open(staged, O_RDWR | O_CLOEXEC);
    if (fd != -1) {
        patch_arm64e(fd);
        close(fd);
    }
    posix_spawn_file_actions_t actions;
    posix_spawn_file_actions_init(&actions);
    posix_spawn_file_actions_addopen(&actions, 1, "/dev/null", O_WRONLY, 0);
    posix_spawn_file_actions_addopen(&actions, 2, "/dev/null", O_WRONLY, 0);
    char *arguments[] = {"/usr/bin/codesign", "-s", "-", "-f", staged, NULL};
    char *empty[] = {NULL};
    pid_t child;
    int status = 0;
    int spawned = posix_spawn(&child, arguments[0], &actions, NULL, arguments, empty);
    posix_spawn_file_actions_destroy(&actions);
    if (spawned != 0 || waitpid(child, &status, 0) == -1 || !WIFEXITED(status) ||
        WEXITSTATUS(status) != 0) {
        unlink(staged);
        return NULL;
    }
    struct timeval times[2] = {
        {original.st_atimespec.tv_sec, (int)(original.st_atimespec.tv_nsec / 1000)},
        {original.st_mtimespec.tv_sec, (int)(original.st_mtimespec.tv_nsec / 1000)},
    };
    utimes(staged, times);
    chmod(staged, original.st_mode & 0755);
    if (rename(staged, copy) == -1) {
        unlink(staged);
        return NULL;
    }
    return copy;
}

/* Keep tracing alive in children even when a build resets the environment. */
static char **traced_environment(char *const envp[]) {
    pthread_once(&once, setup);
    const char *names[] = {"DYLD_INSERT_LIBRARIES", "DARWINTRACE_LOG", "DARWINTRACE_PREFIX",
                           "DARWINTRACE_SIPDIR"};
    const char *values[] = {self_path, log_path, prefix, sipdir};
    size_t count = 0;
    while (envp && envp[count]) {
        count++;
    }
    char **result = calloc(count + 5, sizeof *result);
    if (!result) {
        return (char **)envp;
    }
    size_t used = 0;
    for (size_t index = 0; index < count; index++) {
        int ours = 0;
        for (int name = 0; name < 4; name++) {
            size_t length = strlen(names[name]);
            if (strncmp(envp[index], names[name], length) == 0 && envp[index][length] == '=') {
                ours = 1;
            }
        }
        if (!ours) {
            result[used++] = envp[index];
        }
    }
    for (int name = 0; name < 4; name++) {
        if (values[name]) {
            char *entry = NULL;
            if (asprintf(&entry, "%s=%s", names[name], values[name]) != -1) {
                result[used++] = entry;
            }
        }
    }
    result[used] = NULL;
    return result;
}

/*
 * The program to really execute and its arguments: a SIP copy of a protected
 * program, or of the protected interpreter of a script.
 */
static const char *rewrite(const char *path, char *const argv[], char ***new_argv, char *copy,
                           size_t size) {
    *new_argv = NULL;
    if (!path) {
        return path;
    }
    if (sip_protected(path)) {
        const char *traced = sip_copy(path, copy, size);
        return traced ? traced : path;
    }
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd == -1) {
        return path;
    }
    char line[PATH_MAX + 64];
    ssize_t length = read(fd, line, sizeof line - 1);
    close(fd);
    if (length < 3 || line[0] != '#' || line[1] != '!') {
        return path;
    }
    line[length] = '\0';
    char *end = strchr(line, '\n');
    if (end) {
        *end = '\0';
    }
    char *interpreter = line + 2;
    while (*interpreter == ' ' || *interpreter == '\t') {
        interpreter++;
    }
    char *argument = interpreter + strcspn(interpreter, " \t");
    if (*argument) {
        *argument++ = '\0';
        while (*argument == ' ' || *argument == '\t') {
            argument++;
        }
    }
    if (!sip_protected(interpreter)) {
        return path;
    }
    const char *traced = sip_copy(interpreter, copy, size);
    if (!traced) {
        return path;
    }
    size_t count = 0;
    while (argv && argv[count]) {
        count++;
    }
    char **arguments = calloc(count + 4, sizeof *arguments);
    if (!arguments) {
        return path;
    }
    size_t used = 0;
    arguments[used++] = strdup(interpreter);
    if (*argument) {
        arguments[used++] = strdup(argument);
    }
    arguments[used++] = strdup(path);
    for (size_t index = 1; index < count; index++) {
        arguments[used++] = argv[index];
    }
    arguments[used] = NULL;
    *new_argv = arguments;
    return traced;
}

static int traced_open(const char *path, int flags, ...) {
    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list list;
        va_start(list, flags);
        mode = (mode_t)va_arg(list, int);
        va_end(list);
    }
    record(AT_FDCWD, path);
    return open(path, flags, mode);
}

static int traced_openat(int directory, const char *path, int flags, ...) {
    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list list;
        va_start(list, flags);
        mode = (mode_t)va_arg(list, int);
        va_end(list);
    }
    record(directory, path);
    return openat(directory, path, flags, mode);
}

static int traced_stat(const char *path, struct stat *buffer) {
    record(AT_FDCWD, path);
    return stat(path, buffer);
}

static int traced_lstat(const char *path, struct stat *buffer) {
    record(AT_FDCWD, path);
    return lstat(path, buffer);
}

static int traced_fstatat(int directory, const char *path, struct stat *buffer, int flags) {
    record(directory, path);
    return fstatat(directory, path, buffer, flags);
}

static int traced_access(const char *path, int mode) {
    record(AT_FDCWD, path);
    return access(path, mode);
}

static int traced_faccessat(int directory, const char *path, int mode, int flags) {
    record(directory, path);
    return faccessat(directory, path, mode, flags);
}

static ssize_t traced_readlink(const char *path, char *buffer, size_t size) {
    record(AT_FDCWD, path);
    return readlink(path, buffer, size);
}

static DIR *traced_opendir(const char *path) {
    record(AT_FDCWD, path);
    return opendir(path);
}

static int traced_getattrlist(const char *path, void *list, void *buffer, size_t size,
                              unsigned int options) {
    record(AT_FDCWD, path);
    return getattrlist(path, list, buffer, size, options);
}

static int traced_execve(const char *path, char *const argv[], char *const envp[]) {
    record(AT_FDCWD, path);
    char copy[PATH_MAX];
    char **arguments;
    const char *program = rewrite(path, argv, &arguments, copy, sizeof copy);
    return execve(program, arguments ? arguments : argv, traced_environment(envp));
}

static int traced_posix_spawn(pid_t *pid, const char *path,
                              const posix_spawn_file_actions_t *actions,
                              const posix_spawnattr_t *attributes, char *const argv[],
                              char *const envp[]) {
    record(AT_FDCWD, path);
    char copy[PATH_MAX];
    char **arguments;
    const char *program = rewrite(path, argv, &arguments, copy, sizeof copy);
    char **environment = traced_environment(envp);
    int result = posix_spawn(pid, program, actions, attributes, arguments ? arguments : argv,
                             environment);
    free(arguments);
    if (environment != (char **)envp) {
        free(environment);
    }
    return result;
}

static int traced_posix_spawnp(pid_t *pid, const char *file,
                               const posix_spawn_file_actions_t *actions,
                               const posix_spawnattr_t *attributes, char *const argv[],
                               char *const envp[]) {
    if (file && !strchr(file, '/')) {
        const char *search = getenv("PATH");
        char candidate[PATH_MAX];
        for (const char *entry = search ? search : "/usr/bin:/bin"; entry && *entry;) {
            const char *end = strchr(entry, ':');
            size_t length = end ? (size_t)(end - entry) : strlen(entry);
            if ((size_t)snprintf(candidate, sizeof candidate, "%.*s/%s", (int)length, entry,
                                 file) < sizeof candidate &&
                access(candidate, X_OK) == 0) {
                return traced_posix_spawn(pid, candidate, actions, attributes, argv, envp);
            }
            entry = end ? end + 1 : NULL;
        }
    }
    return traced_posix_spawn(pid, file, actions, attributes, argv, envp);
}

#define INTERPOSE(replacement, original)                                                         \
    __attribute__((used)) static struct {                                                         \
        const void *new;                                                                          \
        const void *old;                                                                          \
    } interpose_##original __attribute__((section("__DATA,__interpose"))) = {                    \
        (const void *)(unsigned long)&replacement, (const void *)(unsigned long)&original}

INTERPOSE(traced_open, open);
INTERPOSE(traced_openat, openat);
INTERPOSE(traced_stat, stat);
INTERPOSE(traced_lstat, lstat);
INTERPOSE(traced_fstatat, fstatat);
INTERPOSE(traced_access, access);
INTERPOSE(traced_faccessat, faccessat);
INTERPOSE(traced_readlink, readlink);
INTERPOSE(traced_opendir, opendir);
INTERPOSE(traced_getattrlist, getattrlist);
INTERPOSE(traced_execve, execve);
INTERPOSE(traced_posix_spawn, posix_spawn);
INTERPOSE(traced_posix_spawnp, posix_spawnp);
