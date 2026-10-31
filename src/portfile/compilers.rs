//! Native PortSystem compiler, SDK and flag selection (portconfigure).
use super::*;
use crate::registry::compare_versions;
use molt::{MoltResult, check_args};
use regex::Regex;
use std::{
    cmp::Ordering,
    collections::HashMap,
    path::Path,
    process::Command,
    sync::{LazyLock, Mutex},
};

const CLT: &str = "/Library/Developer/CommandLineTools";

pub(super) fn install(interp: &mut Interp, id: ContextID) -> MoltResult {
    for (name, command) in [
        (
            "portconfigure::configure_get_compiler",
            get_compiler as molt::CommandFunc,
        ),
        (
            "portconfigure::configure_get_default_compiler",
            default_compiler,
        ),
        ("portconfigure::get_compiler_fallback", compiler_fallback),
        ("portconfigure::get_fortran_fallback", fortran_fallback),
        (
            "portconfigure::configure_get_fortran_compiler",
            fortran_compiler,
        ),
        ("portconfigure::compiler_port_name", port_name),
        ("portconfigure::compiler_is_port", is_port),
        ("portconfigure::arch_flag_supported", arch_flag),
        ("portconfigure::configure_get_archflags", archflags),
        ("portconfigure::configure_get_ld_archflags", ld_archflags),
        (
            "portconfigure::configure_get_universal_archflags",
            universal_archflags,
        ),
        ("portconfigure::configure_get_sdkroot", sdkroot),
        ("portconfigure::configure_get_cxx_stdlib", cxx_stdlib),
        ("portconfigure::construct_cxxflags", cxxflags),
        ("portconfigure::should_add_stdlib", add_stdlib),
        ("portconfigure::should_add_cxx_abi", add_cxx_abi),
        ("portconfigure::configure_get_cppflags", cppflags),
        ("portconfigure::configure_get_ldflags", ldflags),
        ("portconfigure::find_developer_tool", developer_tool),
        ("compiler.command_line_tools_version", tools_version),
        ("get_compiler_version", compiler_version),
        ("get_tool_path", tool_path),
        (
            "portconfigure::load_implicit_function_declaration_whitelist",
            whitelist,
        ),
    ] {
        interp.add_context_command(name, command, id);
    }
    for name in [
        "compiler.blacklist",
        "compiler.require_fortran",
        "compiler.fortran_fallback",
        "compiler.limit_flags",
        "compiler.support_environment_paths",
        "compiler.support_environment_sdkroot",
        "configure.build_arch",
        "configure.ld_archflags",
        "configure.universal_cflags",
        "configure.universal_cxxflags",
        "configure.universal_objcflags",
        "configure.universal_objcxxflags",
        "configure.universal_cppflags",
        "configure.universal_ldflags",
    ] {
        commands::register(interp, id, name);
    }
    let mut defaults = vec![
        ("compiler.require_fortran", "no".to_string()),
        (
            "compiler.fortran_fallback",
            "[portconfigure::get_fortran_fallback]".into(),
        ),
        ("compiler.limit_flags", "no".into()),
        ("compiler.support_environment_paths", "no".into()),
        ("compiler.support_environment_sdkroot", "no".into()),
        (
            "compiler.c_standard",
            "[expr {$supported_archs ne \"noarch\" ? 1989 : \"\"}]".into(),
        ),
        (
            "compiler.cxx_standard",
            "[expr {$supported_archs ne \"noarch\" ? 1998 : \"\"}]".into(),
        ),
        ("compiler.openmp_version", String::new()),
        ("compiler.mpi", String::new()),
        ("compiler.thread_local_storage", "no".into()),
        ("compiler.blacklist", String::new()),
        ("compiler.whitelist", String::new()),
        (
            "compiler.fallback",
            "[portconfigure::get_compiler_fallback]".into(),
        ),
        (
            "configure.compiler",
            "[portconfigure::configure_get_default_compiler]".into(),
        ),
        ("configure.compiler.add_deps", "yes".into()),
        (
            "configure.cxx_stdlib",
            "[portconfigure::configure_get_cxx_stdlib]".into(),
        ),
        ("configure.optflags", "-Os".into()),
        ("configure.cflags", "${configure.optflags}".into()),
        ("configure.objcflags", "${configure.optflags}".into()),
        (
            "configure.cxxflags",
            "[portconfigure::construct_cxxflags ${configure.optflags}]".into(),
        ),
        ("configure.objcxxflags", "${configure.cxxflags}".into()),
        (
            "configure.cppflags",
            "[portconfigure::configure_get_cppflags]".into(),
        ),
        (
            "configure.ldflags",
            "[portconfigure::configure_get_ldflags]".into(),
        ),
        ("configure.libs", String::new()),
        ("configure.fflags", "${configure.optflags}".into()),
        ("configure.f90flags", "${configure.optflags}".into()),
        ("configure.fcflags", "${configure.optflags}".into()),
        ("configure.classpath", String::new()),
        ("configure.perl", String::new()),
        ("configure.python", String::new()),
        ("configure.ruby", String::new()),
        (
            "configure.install",
            "${portutil::autoconf::install_command}".into(),
        ),
        (
            "portutil::autoconf::install_command",
            "/usr/bin/install -c".into(),
        ),
        ("configure.awk", String::new()),
        ("configure.bison", String::new()),
        ("configure.pkg_config", String::new()),
        ("configure.pkg_config_path", String::new()),
        ("configure.asroot", "no".into()),
        (
            "configure.ld_archflags",
            "[portconfigure::configure_get_ld_archflags]".into(),
        ),
        (
            "configure.sdkroot",
            "[portconfigure::configure_get_sdkroot ${configure.sdk_version}]".into(),
        ),
        (
            "configure.sysroot",
            "[expr {${configure.sdkroot} ne \"\" ? ${configure.sdkroot} : \"/\"}]".into(),
        ),
        (
            "configure.universal_cflags",
            "[portconfigure::configure_get_universal_archflags]".into(),
        ),
        (
            "configure.universal_objcflags",
            "${configure.universal_cflags}".into(),
        ),
        (
            "configure.universal_cxxflags",
            "[portconfigure::configure_get_universal_archflags]".into(),
        ),
        (
            "configure.universal_objcxxflags",
            "${configure.universal_cxxflags}".into(),
        ),
        ("configure.universal_cppflags", String::new()),
        (
            "configure.universal_ldflags",
            "[portconfigure::configure_get_universal_archflags]".into(),
        ),
        ("configure.ccache", "${configureccache}".into()),
        ("configure.distcc", "${configuredistcc}".into()),
        ("configure.pipe", "${configurepipe}".into()),
        ("configure.nice", "${buildnicevalue}".into()),
        (
            "configure.checks.implicit_function_declaration.whitelist",
            "[portconfigure::load_implicit_function_declaration_whitelist ${configure.sdk_version}]"
                .into(),
        ),
        ("use_xcode", "[portmain::use_xcode]".into()),
        (
            "configure.developer_dir",
            "[portconfigure::configure_get_developer_dir]".into(),
        ),
    ];
    for tool in [
        "cc", "cxx", "objc", "objcxx", "cpp", "f77", "f90", "fc", "javac",
    ] {
        defaults.push((
            leak(format!("configure.{tool}")),
            format!("[portconfigure::configure_get_compiler {tool}]"),
        ));
    }
    for tool in ["cc", "cxx", "objc", "objcxx", "f77", "f90", "fc"] {
        defaults.push((
            leak(format!("configure.{tool}_archflags")),
            format!("[portconfigure::configure_get_archflags {tool}]"),
        ));
    }
    for (name, value) in [
        ("configureccache", "no"),
        ("configuredistcc", "no"),
        ("configurepipe", "yes"),
        ("buildnicevalue", "0"),
        ("cxx_stdlib", "libc++"),
    ] {
        if !interp.var_exists(&name.into()) {
            interp.set_scalar(name, value.into())?;
        }
    }
    for (name, value) in defaults {
        defaults::register(interp, id, name, &value)?;
        interp
            .context::<State>(id)
            .execution_defaults
            .insert(name.into());
    }
    Ok(Value::empty())
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn get(interp: &mut Interp, name: &str) -> Result<String, Exception> {
    Ok(interp.scalar(name)?.to_string())
}

fn optional(interp: &mut Interp, name: &str) -> Result<Option<String>, Exception> {
    if interp.var_exists(&name.into()) {
        get(interp, name).map(Some)
    } else {
        Ok(None)
    }
}

fn flag(interp: &mut Interp, name: &str) -> Result<bool, Exception> {
    Ok(match optional(interp, name)? {
        Some(value) if !value.is_empty() => Value::from(value).as_bool()?,
        _ => false,
    })
}

fn number(interp: &mut Interp, name: &str) -> Result<Option<i64>, Exception> {
    Ok(optional(interp, name)?.and_then(|value| value.trim().parse().ok()))
}

fn at_least(interp: &mut Interp, name: &str, threshold: i64) -> Result<bool, Exception> {
    Ok(number(interp, name)?.is_some_and(|value| value >= threshold))
}

fn vercmp(a: &str, b: &str) -> Ordering {
    compare_versions(a, b)
}

fn max_version(a: String, b: &str) -> String {
    if vercmp(&a, b).is_ge() { a } else { b.into() }
}

fn executable(path: &str) -> bool {
    !path.is_empty() && filesystem::predicate("executable", path)
}

fn list(items: Vec<String>) -> Value {
    Value::from(items.into_iter().map(Value::from).collect::<Vec<_>>())
}

fn os_major(interp: &mut Interp) -> Result<i64, Exception> {
    Ok(number(interp, "os.major")?.unwrap_or(0))
}

// ---------------------------------------------------------------------------
// Host tool discovery, cached per process like upstream's macports caches.

static TOOL_PATHS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);
static COMPILER_VERSIONS: LazyLock<Mutex<HashMap<(String, String), String>>> =
    LazyLock::new(Default::default);
static SDK_PATHS: LazyLock<Mutex<HashMap<(String, bool), String>>> =
    LazyLock::new(Default::default);

fn find_tool(tool: &str) -> String {
    if let Some(path) = TOOL_PATHS.lock().unwrap().get(tool) {
        return path.clone();
    }
    let direct = format!("/usr/bin/{tool}");
    let path = if executable(&direct) {
        direct
    } else {
        Command::new("/usr/bin/xcrun")
            .args(["-find", tool])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_default()
    };
    TOOL_PATHS.lock().unwrap().insert(tool.into(), path.clone());
    path
}

fn developer_tool_path(interp: &mut Interp, name: &str) -> Result<String, Exception> {
    let path = find_tool(name);
    if !path.is_empty() {
        return Ok(path);
    }
    Ok(format!("{}/usr/bin/{name}", get(interp, "developer_dir")?))
}

fn compiler_build_version(compiler: &str, developer_dir: &str) -> Result<String, Exception> {
    let key = (compiler.to_string(), developer_dir.to_string());
    if let Some(version) = COMPILER_VERSIONS.lock().unwrap().get(&key) {
        return Ok(version.clone());
    }
    if !executable(compiler) {
        COMPILER_VERSIONS.lock().unwrap().insert(key, String::new());
        return Ok(String::new());
    }
    let pattern = match Path::new(compiler)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
    {
        "clang" => r"clang(?:_.*)?-([0-9.]+)",
        "llvm-gcc-4.2" => r"LLVM build ([0-9.]+)",
        "gcc-4.2" | "gcc-4.0" => r"build ([0-9.]+)",
        _ => {
            return Err(error(format!(
                "don't know how to determine build number of compiler \"{compiler}\""
            )));
        }
    };
    let output = Command::new("/usr/bin/env")
        .arg(format!("DEVELOPER_DIR={developer_dir}"))
        .arg(compiler)
        .arg("-v")
        .output();
    let version = match output {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            Regex::new(pattern)
                .unwrap()
                .captures(&text)
                .and_then(|captures| captures.get(1))
                .map(|found| found.as_str().to_string())
                .ok_or_else(|| {
                    error(format!(
                        "couldn't determine build number of compiler \"{compiler}\""
                    ))
                })?
        }
        Err(_) => String::new(),
    };
    COMPILER_VERSIONS
        .lock()
        .unwrap()
        .insert(key, version.clone());
    Ok(version)
}

fn xcrun_sdk(version: &str, clt: bool) -> String {
    let key = (version.to_string(), clt);
    if let Some(path) = SDK_PATHS.lock().unwrap().get(&key) {
        return path.clone();
    }
    let mut command = Command::new("/usr/bin/xcrun");
    if clt {
        command.env("DEVELOPER_DIR", CLT);
    }
    let path = command
        .args(["--sdk", &format!("macosx{version}"), "--show-sdk-path"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_default();
    SDK_PATHS.lock().unwrap().insert(key, path.clone());
    path
}

// ---------------------------------------------------------------------------
// Compiler naming and paths.

static PORT_NAMES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"^apple-gcc-(\d+)\.(\d+)$", "apple-gcc%s%s"),
        (r"^macports-clang-(\d+(?:\.\d+)?)$", "clang-%s"),
        (r"^macports-(llvm-)?gcc-(\d+)(?:\.(\d+))?$", "%sgcc%s%s"),
        (
            r"^macports-(mpich|openmpi|mpich-devel|openmpi-devel)-default$",
            "%s-default",
        ),
        (
            r"^macports-(mpich|openmpi|mpich-devel|openmpi-devel)-clang$",
            "%s-clang",
        ),
        (
            r"^macports-(mpich|openmpi|mpich-devel|openmpi-devel)-clang-(\d+)\.(\d+)$",
            "%s-clang%s%s",
        ),
        (
            r"^macports-(mpich|openmpi|mpich-devel|openmpi-devel)-clang-(\d+)$",
            "%s-clang%s",
        ),
        (
            r"^macports-(mpich|openmpi|mpich-devel|openmpi-devel)-gcc-(\d+)(?:\.(\d+))?$",
            "%s-gcc%s%s",
        ),
        (r"^macports-g95$", "g95"),
        (r"^macports-(clang|gcc)-devel$", "%s-devel"),
    ]
    .into_iter()
    .map(|(pattern, format)| (Regex::new(pattern).unwrap(), format))
    .collect()
});

pub(super) fn compiler_port(compiler: &str) -> String {
    for (pattern, format) in PORT_NAMES.iter() {
        if let Some(captures) = pattern.captures(compiler) {
            let mut result = String::new();
            let mut groups = captures
                .iter()
                .skip(1)
                .map(|group| group.map(|found| found.as_str()).unwrap_or(""));
            let mut parts = format.split("%s").peekable();
            while let Some(part) = parts.next() {
                result.push_str(part);
                if parts.peek().is_some() {
                    result.push_str(groups.next().unwrap_or(""));
                }
            }
            return result;
        }
    }
    String::new()
}

fn supports_arch_flag(
    interp: &mut Interp,
    compiler: &str,
    multiple: bool,
) -> Result<bool, Exception> {
    if multiple {
        return Ok(Regex::new(r"^gcc-4|llvm|apple|clang")
            .unwrap()
            .is_match(compiler));
    }
    let old_gcc = Regex::new(r"^macports(?:-[^-]+)?-gcc-4\.[0-6]").unwrap();
    let subplatform = optional(interp, "os.subplatform")?.unwrap_or_default();
    Ok(!(old_gcc.is_match(compiler)
        || (subplatform != "macosx" && matches!(compiler, "cc" | "gcc"))))
}

/// `portconfigure::configure_get_compiler`; an empty compiler selects the
/// configured one (or the Fortran compiler for Fortran tools when required).
fn compiler_path(interp: &mut Interp, kind: &str, compiler: &str) -> Result<String, Exception> {
    let compiler = if !compiler.is_empty() {
        compiler.to_string()
    } else if flag(interp, "compiler.require_fortran")? && matches!(kind, "fc" | "f77" | "f90") {
        fortran_choice(interp)?
    } else {
        get(interp, "configure.compiler")?
    };
    let prefix = optional(interp, "prefix_frozen")?
        .or(optional(interp, "prefix")?)
        .unwrap_or_else(|| "/opt/local".into());
    let major = os_major(interp)?;
    let c_like = matches!(kind, "cc" | "objc");
    let cxx_like = matches!(kind, "cxx" | "objcxx");
    let fortran = matches!(kind, "fc" | "f77" | "f90");
    let captures = |pattern: &str| {
        Regex::new(pattern).unwrap().captures(&compiler).map(|c| {
            c.iter()
                .skip(1)
                .map(|group| {
                    group
                        .map(|found| found.as_str().to_string())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        })
    };
    let bin = |name: String| Some(format!("{prefix}/bin/{name}"));
    let path = if let Some(groups) = captures(r"^apple-gcc(-4\.[02])$") {
        let suffix = &groups[0];
        if c_like {
            bin(format!("gcc-apple{suffix}"))
        } else if cxx_like && suffix == "-4.2" {
            bin(format!("g++-apple{suffix}"))
        } else if kind == "cpp" {
            bin(format!("cpp-apple{suffix}"))
        } else {
            None
        }
    } else if compiler == "clang" {
        if c_like {
            Some(developer_tool_path(interp, "clang")?)
        } else if cxx_like {
            let clangpp = developer_tool_path(interp, "clang++")?;
            if executable(&clangpp) {
                Some(clangpp)
            } else if major <= 12 {
                Some(developer_tool_path(interp, "llvm-g++-4.2")?)
            } else {
                return Ok(String::new());
            }
        } else {
            None
        }
    } else if let Some(groups) = captures(r"^gcc(-3\.3|-4\.[02])?$") {
        if major >= 12 {
            return Ok(String::new());
        }
        let suffix = &groups[0];
        match kind {
            _ if c_like => Some(developer_tool_path(interp, &format!("gcc{suffix}"))?),
            _ if cxx_like => Some(developer_tool_path(interp, &format!("g++{suffix}"))?),
            "cpp" => Some(developer_tool_path(interp, &format!("cpp{suffix}"))?),
            _ => None,
        }
    } else if compiler == "llvm-gcc-4.2" {
        if major >= 13 {
            return Ok(String::new());
        }
        match kind {
            _ if c_like => Some(developer_tool_path(interp, "llvm-gcc-4.2")?),
            _ if cxx_like => Some(developer_tool_path(interp, "llvm-g++-4.2")?),
            "cpp" => Some(developer_tool_path(interp, "llvm-cpp-4.2")?),
            _ => None,
        }
    } else if let Some(groups) = captures(r"^macports-clang(-[0-3]\.\d+)?$") {
        let suffix = if groups[0].is_empty() {
            String::new()
        } else {
            format!("-mp{}", groups[0])
        };
        if c_like {
            bin(format!("clang{suffix}"))
        } else if cxx_like {
            bin(format!("clang++{suffix}"))
        } else {
            None
        }
    } else if let Some(groups) = captures(r"^macports-clang(-\d+(?:\.\d+)?)$") {
        let suffix = format!("-mp{}", groups[0]);
        match kind {
            _ if c_like => bin(format!("clang{suffix}")),
            _ if cxx_like => bin(format!("clang++{suffix}")),
            "cpp" => bin(format!("clang-cpp{suffix}")),
            _ => None,
        }
    } else if let Some(groups) = captures(r"^macports-gcc(-\d+(?:\.\d+)?)?$") {
        let suffix = if groups[0].is_empty() {
            String::new()
        } else {
            format!("-mp{}", groups[0])
        };
        match kind {
            _ if c_like => bin(format!("gcc{suffix}")),
            _ if cxx_like => bin(format!("g++{suffix}")),
            "cpp" => bin(format!("cpp{suffix}")),
            _ if fortran => bin(format!("gfortran{suffix}")),
            _ => None,
        }
    } else if let Some(groups) = captures(r"^macports-(clang|gcc)-devel$") {
        let clang = groups[0] == "clang";
        match kind {
            _ if c_like => bin(format!("{}-mp-devel", if clang { "clang" } else { "gcc" })),
            _ if cxx_like => bin(format!(
                "{}-mp-devel",
                if clang { "clang++" } else { "g++" }
            )),
            "cpp" => bin(format!(
                "{}-mp-devel",
                if clang { "clang-cpp" } else { "cpp" }
            )),
            _ if fortran && !clang => bin("gfortran-mp-devel".into()),
            _ => None,
        }
    } else if compiler == "macports-llvm-gcc-4.2" {
        match kind {
            _ if c_like => bin("llvm-gcc-4.2".into()),
            _ if cxx_like => bin("llvm-g++-4.2".into()),
            "cpp" => bin("llvm-cpp-4.2".into()),
            _ => None,
        }
    } else if compiler == "macports-g95" {
        fortran.then(|| format!("{prefix}/bin/g95"))
    } else if let Some(groups) =
        captures(r"^macports-(mpich|openmpi|mpich-devel|openmpi-devel)-clang$")
    {
        let mpi = &groups[0];
        match kind {
            _ if c_like => bin(format!("mpicc-{mpi}-clang")),
            _ if cxx_like => bin(format!("mpicxx-{mpi}-clang")),
            _ => None,
        }
    } else if let Some(groups) =
        captures(r"^macports-(mpich|openmpi|mpich-devel|openmpi-devel)-clang-(\d+(?:\.\d+)?)$")
    {
        let (mpi, version) = (&groups[0], &groups[1]);
        let suffix = version.replace('.', "");
        match kind {
            _ if c_like => bin(format!("mpicc-{mpi}-clang{suffix}")),
            _ if cxx_like => bin(format!("mpicxx-{mpi}-clang{suffix}")),
            "cpp" => bin(format!("clang-cpp-mp-{version}")),
            _ => None,
        }
    } else if let Some(groups) =
        captures(r"^macports-(mpich|openmpi|mpich-devel|openmpi-devel)-gcc-(\d+(?:\.\d+)?)$")
    {
        let (mpi, version) = (&groups[0], &groups[1]);
        let suffix = version.replace('.', "");
        match kind {
            _ if c_like => bin(format!("mpicc-{mpi}-gcc{suffix}")),
            _ if cxx_like => bin(format!("mpicxx-{mpi}-gcc{suffix}")),
            "cpp" => bin(format!("cpp-mp-{version}")),
            _ if fortran => bin(format!("mpifort-{mpi}-gcc{suffix}")),
            _ => None,
        }
    } else if let Some(groups) =
        captures(r"^macports-(mpich|openmpi|mpich-devel|openmpi-devel)-default$")
    {
        let mpi = &groups[0];
        match kind {
            _ if c_like => bin(format!("mpicc-{mpi}-mp")),
            _ if cxx_like => bin(format!("mpicxx-{mpi}-mp")),
            _ => None,
        }
    } else {
        None
    };
    if let Some(path) = path {
        return Ok(path);
    }
    Ok(match kind {
        _ if c_like => developer_tool_path(interp, "cc")?,
        _ if cxx_like => developer_tool_path(interp, "c++")?,
        "cpp" => developer_tool_path(interp, "cpp")?,
        _ => String::new(),
    })
}

fn tools_version_of(interp: &mut Interp, compiler: &str) -> Result<String, Exception> {
    let cc = compiler_path(interp, "cc", compiler)?;
    if cc.is_empty() {
        return Ok(String::new());
    }
    let developer = get(interp, "configure.developer_dir")?;
    compiler_build_version(&cc, &developer)
}

// ---------------------------------------------------------------------------
// Fallback lists.

fn min_command_line(interp: &mut Interp, compiler: &str) -> Result<Option<String>, Exception> {
    let major = os_major(interp)?;
    let stdlib = get(interp, "configure.cxx_stdlib")?;
    let tls = flag(interp, "compiler.thread_local_storage")?;
    if !get(interp, "compiler.openmp_version")?.is_empty()
        || (tls && major < 11)
        || stdlib == "macports-libstdc++"
    {
        return Ok(None);
    }
    let limit = flag(interp, "compiler.limit_flags")?;
    let env_paths = flag(interp, "compiler.support_environment_paths")?;
    let env_sdkroot = flag(interp, "compiler.support_environment_sdkroot")?;
    let sdkroot = get(interp, "configure.sdkroot")?;
    let mut min = String::from("1.0");
    match compiler {
        "clang" => {
            if stdlib == "libc++" && major < 11 {
                return Ok(None);
            }
            if (limit || env_sdkroot)
                && !sdkroot.is_empty()
                && !(Path::new("/usr/lib/libxcselect.dylib").exists() || major >= 20)
            {
                return Ok(None);
            }
            if at_least(interp, "compiler.c_standard", 2017)? {
                min = max_version(min, "1000.11.45.2");
            } else if at_least(interp, "compiler.c_standard", 2011)? {
                min = max_version(min, "500.2.75");
            }
            if at_least(interp, "compiler.cxx_standard", 2020)? {
                min = max_version(min, "1500");
            } else if at_least(interp, "compiler.cxx_standard", 2017)? {
                min = max_version(min, "1000.11.45.2");
            } else if at_least(interp, "compiler.cxx_standard", 2014)? {
                min = max_version(min, "602.0.49");
            } else if at_least(interp, "compiler.cxx_standard", 2011)? {
                min = max_version(min, "500.2.75");
            }
            if at_least(interp, "compiler.cxx_standard", 2011)? && tls {
                min = max_version(min, "800.0.38");
            }
            if limit || env_paths {
                min = max_version(min, "421.0.57");
            }
        }
        "llvm-gcc-4.2" | "gcc-4.2" | "gcc-4.0" | "apple-gcc-4.2" => {
            if number(interp, "compiler.c_standard")?.is_some_and(|value| value > 1999)
                || at_least(interp, "compiler.cxx_standard", 2011)?
                || stdlib == "libc++"
                || tls
            {
                return Ok(None);
            }
            if (limit || env_sdkroot) && !sdkroot.is_empty() {
                return Ok(None);
            }
        }
        other => return Err(error(format!("don't recognize compiler \"{other}\""))),
    }
    Ok(Some(min))
}

fn openmp_at_least(interp: &mut Interp, version: &str) -> Result<bool, Exception> {
    let openmp = get(interp, "compiler.openmp_version")?;
    Ok(vercmp(&openmp, version).is_ge())
}

fn min_clang(interp: &mut Interp) -> Result<String, Exception> {
    let stdlib = get(interp, "configure.cxx_stdlib")?;
    let mut min = String::from("1.0");
    if at_least(interp, "compiler.c_standard", 2017)? {
        min = max_version(min, "6.0");
    } else if at_least(interp, "compiler.c_standard", 2011)? {
        min = max_version(min, "3.1");
    }
    let libcxx = stdlib == "libc++";
    if at_least(interp, "compiler.cxx_standard", 2020)? {
        min = max_version(min, "16");
    } else if at_least(interp, "compiler.cxx_standard", 2017)? {
        min = max_version(min, "5.0");
    } else if at_least(interp, "compiler.cxx_standard", 2014)? {
        min = max_version(min, if libcxx { "3.4" } else { "5.0" });
    } else if at_least(interp, "compiler.cxx_standard", 2011)? {
        min = max_version(min, if libcxx { "3.3" } else { "5.0" });
    }
    if openmp_at_least(interp, "4.0")? {
        min = max_version(min, "6.0");
    } else if openmp_at_least(interp, "2.5")? {
        min = max_version(min, "3.8");
    }
    if flag(interp, "compiler.thread_local_storage")? {
        min = max_version(min, "5.0");
    }
    Ok(min)
}

fn sdk_restricted(interp: &mut Interp) -> Result<bool, Exception> {
    Ok((flag(interp, "compiler.limit_flags")?
        || flag(interp, "compiler.support_environment_sdkroot")?)
        && !get(interp, "configure.sdkroot")?.is_empty())
}

fn min_gcc(interp: &mut Interp) -> Result<Option<String>, Exception> {
    if get(interp, "configure.cxx_stdlib")? == "libc++" {
        return Ok(None);
    }
    let mut min = String::from("1.0");
    if at_least(interp, "compiler.c_standard", 2017)? {
        min = max_version(min, "8");
    } else if at_least(interp, "compiler.c_standard", 2011)? {
        min = max_version(min, "4.3");
    } else if at_least(interp, "compiler.c_standard", 1999)? {
        min = max_version(min, "4.0");
    }
    if at_least(interp, "compiler.cxx_standard", 2020)? {
        min = max_version(min, "12");
    } else if at_least(interp, "compiler.cxx_standard", 2017)? {
        min = max_version(min, "7");
    } else if at_least(interp, "compiler.cxx_standard", 2014)? {
        min = max_version(min, "5");
    } else if at_least(interp, "compiler.cxx_standard", 2011)? {
        min = max_version(min, "4.8");
    }
    min = gcc_openmp(interp, min, true)?;
    Ok(Some(min))
}

fn gcc_openmp(interp: &mut Interp, mut min: String, gcc: bool) -> Result<String, Exception> {
    if openmp_at_least(interp, "4.5")? {
        min = max_version(min, "8");
    } else if openmp_at_least(interp, "4.0")? {
        min = max_version(min, "4.9");
    } else if openmp_at_least(interp, "3.1")? {
        min = max_version(min, "4.7");
    } else if openmp_at_least(interp, "2.5")? {
        min = max_version(min, "4.4");
    }
    let _ = gcc;
    if flag(interp, "compiler.thread_local_storage")? {
        min = max_version(min, "4.5");
    }
    if sdk_restricted(interp)? {
        min = max_version(min, "7");
    }
    Ok(min)
}

fn apple_compilers(interp: &mut Interp) -> Result<Vec<String>, Exception> {
    let xcode = optional(interp, "xcodeversion")?.unwrap_or_default();
    let names: Vec<&str> = if xcode.is_empty() || xcode == "none" {
        let major = os_major(interp)?;
        let candidates: &[&str] = if major >= 13 {
            &["clang"]
        } else if major >= 12 {
            &["clang", "llvm-gcc-4.2"]
        } else if major >= 11 {
            &["clang", "llvm-gcc-4.2", "gcc-4.2"]
        } else {
            &[]
        };
        let mut found = Vec::new();
        for name in candidates {
            if executable(&developer_tool_path(interp, name)?) {
                found.push(*name);
            }
        }
        found
    } else if vercmp(&xcode, "5").is_ge() {
        vec!["clang"]
    } else if vercmp(&xcode, "4.3").is_ge() {
        vec!["clang", "llvm-gcc-4.2"]
    } else if vercmp(&xcode, "4.2").is_ge() {
        vec!["llvm-gcc-4.2", "clang"]
    } else if vercmp(&xcode, "4.0").is_ge() {
        vec!["llvm-gcc-4.2", "clang", "gcc-4.2"]
    } else if get(interp, "configure.sdkroot")?.contains("10.4u") {
        vec!["gcc-4.0"]
    } else if vercmp(&xcode, "3.2").is_ge() {
        vec!["gcc-4.2", "llvm-gcc-4.2", "clang", "gcc-4.0"]
    } else if vercmp(&xcode, "3.1").is_ge() {
        vec!["gcc-4.2", "llvm-gcc-4.2", "apple-gcc-4.2", "gcc-4.0"]
    } else {
        vec!["apple-gcc-4.2", "gcc-4.0"]
    };
    Ok(names.into_iter().map(String::from).collect())
}

/// Source a ports-tree compiler list in procedure scope, as upstream does, or
/// use the built-in selection when the tree does not provide one.
fn tree_compilers(
    interp: &mut Interp,
    id: ContextID,
    file: &str,
) -> Result<Option<Vec<String>>, Exception> {
    let Some(root) = interp
        .context::<State>(id)
        .groups
        .resource_directory
        .clone()
    else {
        return Ok(None);
    };
    let path = root.join("port1.0/compilers").join(file);
    if !path.is_file() {
        return Ok(None);
    }
    let source = std::fs::read_to_string(&path)
        .map_err(|err| error(format!("cannot read {}: {err}", path.display())))?;
    // Evaluate as an inner procedure so a top-level `return` ends only the
    // resource, as `source` does; the session itself cannot source files.
    let inner = Value::from(vec![
        Value::from(""),
        Value::from(format!(
            "upvar 1 compilers compilers\nglobal os.major os.platform porturl\n{source}"
        )),
        Value::from("portconfigure"),
    ]);
    let script = Value::from(vec![
        Value::from("apply"),
        Value::from(vec![
            Value::from(vec![Value::from("inner")]),
            Value::from("set compilers [list]; apply $inner; return $compilers"),
            Value::from("portconfigure"),
        ]),
        inner,
    ]);
    let result = interp.eval_value(&script)?;
    Ok(Some(
        result.as_list()?.iter().map(Value::to_string).collect(),
    ))
}

fn clang_compilers(interp: &mut Interp, id: ContextID) -> Result<Vec<String>, Exception> {
    if let Some(compilers) = tree_compilers(interp, id, "clang_compilers.tcl")? {
        return Ok(compilers);
    }
    let major = os_major(interp)?;
    let darwin = optional(interp, "os.platform")?.unwrap_or_default() == "darwin";
    let mut compilers = Vec::new();
    if major >= 11 || !darwin {
        if at_least(interp, "compiler.cxx_standard", 2014)? && major >= 22 {
            compilers.push("macports-clang-17");
        }
        compilers.extend([
            "macports-clang-16",
            "macports-clang-15",
            "macports-clang-14",
        ]);
        if major < 23 || !darwin {
            compilers.extend(["macports-clang-13", "macports-clang-12"]);
        }
    }
    if darwin {
        if major >= 10 {
            compilers.push("macports-clang-11");
            if optional(interp, "build_arch")?.unwrap_or_default() != "arm64" {
                compilers.extend(["macports-clang-10", "macports-clang-9.0"]);
                if major < 20 {
                    compilers.push("macports-clang-8.0");
                }
            }
        }
        if major < 20 {
            compilers.extend([
                "macports-clang-7.0",
                "macports-clang-6.0",
                "macports-clang-5.0",
            ]);
        }
        if major < 16 {
            compilers.extend(["macports-clang-3.7", "macports-clang-3.4"]);
        }
    }
    Ok(compilers.into_iter().map(String::from).collect())
}

fn gcc_compilers(interp: &mut Interp, id: ContextID) -> Result<Vec<String>, Exception> {
    if let Some(compilers) = tree_compilers(interp, id, "gcc_compilers.tcl")? {
        return Ok(compilers);
    }
    let major = os_major(interp)?;
    let darwin = optional(interp, "os.platform")?.unwrap_or_default() == "darwin";
    let mut compilers = Vec::new();
    if major >= 10 || !darwin {
        compilers.extend([
            "macports-gcc-13",
            "macports-gcc-12",
            "macports-gcc-11",
            "macports-gcc-10",
        ]);
    }
    if major < 15 {
        if major >= 10 {
            compilers.extend(["macports-gcc-9", "macports-gcc-8"]);
        }
        compilers.extend(["macports-gcc-7", "macports-gcc-6", "macports-gcc-5"]);
    }
    if major >= 10 {
        compilers.push("macports-gcc-devel");
    }
    Ok(compilers.into_iter().map(String::from).collect())
}

fn version_of(compiler: &str) -> &str {
    compiler.split('-').nth(2).unwrap_or("")
}

fn mpi_wrapper(mpi: &str, compiler: &str) -> String {
    let parts: Vec<&str> = compiler.split('-').collect();
    if parts.first() != Some(&"macports") {
        return format!("macports-{mpi}-default");
    }
    let kind = parts.get(1).copied().unwrap_or("");
    let version = parts.get(2).copied().unwrap_or("");
    if (kind == "clang" && vercmp(version, "3.3").is_lt())
        || (kind == "gcc" && vercmp(version, "4.3").is_lt())
        || kind == "g95"
    {
        return String::new();
    }
    format!("macports-{mpi}-{kind}-{version}")
}

fn with_mpi(interp: &mut Interp, compilers: Vec<String>) -> Result<Vec<String>, Exception> {
    let mpis = optional(interp, "compiler.mpi")?.unwrap_or_default();
    let mpis = Value::from(mpis).as_list()?.to_vec();
    if mpis.is_empty() {
        return Ok(compilers);
    }
    Ok(mpis
        .iter()
        .flat_map(|mpi| compilers.iter().map(move |c| mpi_wrapper(mpi.as_str(), c)))
        .collect())
}

fn fallback_list(interp: &mut Interp, id: ContextID) -> Result<Vec<String>, Exception> {
    if let Some(defaults) = optional(interp, "default_compilers")? {
        return Ok(Value::from(defaults)
            .as_list()?
            .iter()
            .map(Value::to_string)
            .collect());
    }
    let mut compilers = Vec::new();
    if optional(interp, "os.subplatform")?.unwrap_or_default() == "macosx" {
        for compiler in apple_compilers(interp)? {
            if let Some(min) = min_command_line(interp, &compiler)? {
                if compiler == "apple-gcc-4.2" {
                    compilers.push(compiler);
                    continue;
                }
                let version = tools_version_of(interp, &compiler)?;
                if vercmp(&min, &version).is_le() {
                    compilers.push(compiler);
                }
            }
        }
    } else {
        compilers.push("cc".into());
    }
    let min = min_clang(interp)?;
    let clang: Vec<String> = clang_compilers(interp, id)?
        .into_iter()
        .filter(|c| vercmp(&min, version_of(c)).is_le())
        .collect();
    let gcc: Vec<String> = match min_gcc(interp)? {
        Some(min) => gcc_compilers(interp, id)?
            .into_iter()
            .filter(|c| vercmp(&min, version_of(c)).is_le())
            .collect(),
        None => Vec::new(),
    };
    let mut arch = get(interp, "configure.build_arch")?;
    if arch.is_empty() {
        arch = optional(interp, "build_arch")?.unwrap_or_default();
    }
    if matches!(arch.as_str(), "ppc" | "ppc64") {
        compilers.extend(gcc);
        compilers.extend(clang);
    } else {
        compilers.extend(clang);
        compilers.extend(gcc);
    }
    with_mpi(interp, compilers)
}

fn blacklisted(interp: &mut Interp, compiler: &str, versioned: bool) -> Result<bool, Exception> {
    let blacklist = Value::from(get(interp, "compiler.blacklist")?);
    for pattern in blacklist.as_list()?.iter() {
        let words = pattern.as_list()?;
        if versioned && words.len() >= 3 && words[0].as_str() == compiler {
            let version = tools_version_of(interp, compiler)?;
            if version.is_empty() {
                return Ok(true);
            }
            let mut allowed = false;
            for pair in words[1..].chunks(2) {
                let (operator, check) = (
                    pair[0].as_str(),
                    pair.get(1).map(Value::as_str).unwrap_or(""),
                );
                let order = vercmp(&version, check);
                let holds = match operator {
                    "<" => order.is_lt(),
                    "<=" => order.is_le(),
                    ">" => order.is_gt(),
                    ">=" => order.is_ge(),
                    "==" => order.is_eq(),
                    "!=" => order.is_ne(),
                    other => return Err(error(format!("unknown vercmp operator \"{other}\""))),
                };
                if !holds {
                    allowed = true;
                    break;
                }
            }
            if !allowed {
                return Ok(true);
            }
        } else {
            let matched = interp.eval_value(&Value::from(vec![
                Value::from("string"),
                Value::from("match"),
                pattern.clone(),
                Value::from(compiler),
            ]))?;
            if matched.as_bool()? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn default_choice(interp: &mut Interp, id: ContextID) -> Result<String, Exception> {
    let whitelist = Value::from(get(interp, "compiler.whitelist")?)
        .as_list()?
        .to_vec();
    let fallback = Value::from(get(interp, "compiler.fallback")?)
        .as_list()?
        .to_vec();
    let search = if whitelist.is_empty() {
        &fallback
    } else {
        &whitelist
    };
    interp.set_scalar("portconfigure::no_default_compiler_allowed", 0.into())?;
    for compiler in search.iter().map(Value::to_string) {
        if blacklisted(interp, &compiler, true)? {
            continue;
        }
        if executable(&compiler_path(interp, "cc", &compiler)?)
            || !compiler_port(&compiler).is_empty()
        {
            return Ok(compiler);
        }
    }
    let _ = id;
    interp.set_scalar("portconfigure::no_default_compiler_allowed", 1.into())?;
    Ok(fallback.first().map(Value::to_string).unwrap_or_default())
}

fn fortran_list(interp: &mut Interp, id: ContextID) -> Result<Vec<String>, Exception> {
    let min = gcc_openmp(interp, "1.0".into(), false)?;
    let mut compilers: Vec<String> = gcc_compilers(interp, id)?
        .into_iter()
        .filter(|c| vercmp(&min, version_of(c)).is_le())
        .collect();
    let xcode = optional(interp, "xcodeversion")?.unwrap_or_default();
    let darwin = optional(interp, "os.platform")?.unwrap_or_default() == "darwin";
    let g95 = !(darwin && vercmp(&xcode, "9.0").is_gt())
        && get(interp, "compiler.openmp_version")?.is_empty()
        && !sdk_restricted(interp)?;
    if g95 {
        compilers.push("macports-g95".into());
    }
    with_mpi(interp, compilers)
}

fn fortran_choice(interp: &mut Interp) -> Result<String, Exception> {
    let configured = get(interp, "configure.compiler")?;
    if !compiler_path(interp, "fc", &configured)?.is_empty() {
        return Ok(configured);
    }
    let fallback = Value::from(get(interp, "compiler.fortran_fallback")?)
        .as_list()?
        .to_vec();
    for compiler in fallback.iter().map(Value::to_string) {
        if blacklisted(interp, &compiler, false)? {
            continue;
        }
        if executable(&compiler_path(interp, "fc", &compiler)?)
            || !compiler_port(&compiler).is_empty()
        {
            return Ok(compiler);
        }
    }
    Ok(fallback.first().map(Value::to_string).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// SDK selection.

fn close_sdk(version: &str, directory: &str) -> Option<String> {
    let major = version.split('.').next().unwrap_or("");
    let mut sdks: Vec<String> = std::fs::read_dir(directory)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            (name.starts_with(&format!("MacOSX{major}")) && name.ends_with(".sdk"))
                .then(|| format!("{directory}/{name}"))
        })
        .collect();
    sdks.sort_by(|a, b| vercmp(a, b));
    sdks.into_iter()
        .find(|sdk| Path::new(sdk).join("usr/include/sys/cdefs.h").exists())
}

fn sdk_path(interp: &mut Interp, version: &str) -> Result<String, Exception> {
    if interp.array_exists("system_options")
        && let Ok(path) = interp.element("system_options", "macosx_sdk_path")
    {
        return Ok(path.to_string());
    }
    if optional(interp, "os.platform")?.unwrap_or_default() != "darwin" {
        return Ok(String::new());
    }
    let major = os_major(interp)?;
    let host_major = optional(interp, "macos_version_major")?.unwrap_or_default();
    let host_version = optional(interp, "macos_version")?.unwrap_or_default();
    if major < 19 && version == host_major && Path::new("/usr/include/sys/cdefs.h").exists() {
        return Ok(String::new());
    }
    let sdk_major: i64 = version
        .split('.')
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let exists = |path: &str| Path::new(path).exists();
    let try_versions = || {
        if sdk_major >= 11 && sdk_major.to_string() == host_major {
            vec![format!("{sdk_major}.0"), host_version.clone()]
        } else {
            vec![version.to_string()]
        }
    };
    if !flag(interp, "use_xcode")? {
        let sdk = format!("{CLT}/SDKs/MacOSX{version}.sdk");
        if exists(&sdk) {
            return Ok(sdk);
        }
        if sdk_major >= 11
            && let Some(sdk) = close_sdk(version, &format!("{CLT}/SDKs"))
        {
            return Ok(sdk);
        }
        for candidate in try_versions() {
            let sdk = xcrun_sdk(&candidate, true);
            if !sdk.is_empty() {
                return Ok(sdk);
            }
        }
        let sdk = format!("{CLT}/SDKs/MacOSX.sdk");
        if exists(&sdk) {
            return Ok(sdk);
        }
        let sdk = xcrun_sdk("", true);
        if !sdk.is_empty() {
            return Ok(sdk);
        }
    }
    let developer = get(interp, "developer_dir")?;
    let xcode = optional(interp, "xcodeversion")?.unwrap_or_default();
    let sdks_dir = if vercmp(&xcode, "4.3").is_lt() {
        format!("{developer}/SDKs")
    } else {
        format!("{developer}/Platforms/MacOSX.platform/Developer/SDKs")
    };
    for directory in [sdks_dir.clone(), format!("{CLT}/SDKs")] {
        let sdk = if version == "10.4" {
            format!("{directory}/MacOSX10.4u.sdk")
        } else {
            format!("{directory}/MacOSX{version}.sdk")
        };
        if exists(&sdk) {
            return Ok(sdk);
        }
        if sdk_major >= 11
            && let Some(sdk) = close_sdk(version, &directory)
        {
            return Ok(sdk);
        }
    }
    for candidate in try_versions() {
        let sdk = xcrun_sdk(&candidate, false);
        if !sdk.is_empty() {
            return Ok(sdk);
        }
    }
    let sdk = format!("{CLT}/SDKs/MacOSX{version}.sdk");
    if exists(&sdk) {
        return Ok(sdk);
    }
    if sdk_major >= 11
        && let Some(sdk) = close_sdk(version, &format!("{CLT}/SDKs"))
    {
        return Ok(sdk);
    }
    let sdk = format!("{sdks_dir}/MacOSX.sdk");
    if exists(&sdk) {
        return Ok(sdk);
    }
    Ok(xcrun_sdk("", false))
}

// ---------------------------------------------------------------------------
// Tcl command wrappers.

fn global<T>(
    interp: &mut Interp,
    body: impl FnOnce(&mut Interp) -> Result<T, Exception>,
) -> Result<T, Exception> {
    interp.with_global_scope(body)
}

fn get_compiler(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 3, "type ?compiler?")?;
    let compiler = argv.get(2).map(Value::to_string).unwrap_or_default();
    global(interp, |interp| {
        compiler_path(interp, argv[1].as_str(), &compiler)
    })
    .map(Value::from)
}

fn default_compiler(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    global(interp, |interp| default_choice(interp, id)).map(Value::from)
}

fn compiler_fallback(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    global(interp, |interp| fallback_list(interp, id)).map(list)
}

fn fortran_fallback(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    global(interp, |interp| fortran_list(interp, id)).map(list)
}

fn fortran_compiler(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    global(interp, fortran_choice).map(Value::from)
}

fn port_name(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "compiler")?;
    Ok(compiler_port(argv[1].as_str()).into())
}

fn is_port(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "compiler")?;
    Ok((!compiler_port(argv[1].as_str()).is_empty()).into())
}

fn arch_flag(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 3, "compiler ?multiple_arch_flags?")?;
    let multiple = match argv.get(2) {
        Some(value) => value.as_bool()?,
        None => false,
    };
    global(interp, |interp| {
        supports_arch_flag(interp, argv[1].as_str(), multiple)
    })
    .map(Value::from)
}

fn archflags(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "tool")?;
    let tool = argv[1].as_str();
    global(interp, |interp| {
        if flag(interp, "configure.m64")? {
            return Ok("-m64".to_string());
        }
        if flag(interp, "configure.m32")? {
            return Ok("-m32".to_string());
        }
        let arch = get(interp, "configure.build_arch")?;
        if arch.is_empty() {
            return Ok(String::new());
        }
        let compiler = get(interp, "configure.compiler")?;
        Ok(
            if supports_arch_flag(interp, &compiler, false)?
                && matches!(tool, "cc" | "cxx" | "objc" | "objcxx")
            {
                format!("-arch {arch}")
            } else if matches!(arch.as_str(), "arm64" | "ppc64" | "x86_64") {
                "-m64".into()
            } else if compiler != "gcc-3.3" {
                "-m32".into()
            } else {
                String::new()
            },
        )
    })
    .map(Value::from)
}

fn ld_archflags(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    global(interp, |interp| {
        let arch = get(interp, "configure.build_arch")?;
        let compiler = get(interp, "configure.compiler")?;
        Ok(
            if !arch.is_empty() && supports_arch_flag(interp, &compiler, false)? {
                format!("-arch {arch}")
            } else {
                String::new()
            },
        )
    })
    .map(Value::from)
}

fn universal_archflags(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    global(interp, |interp| {
        let archs = Value::from(get(interp, "configure.universal_archs")?);
        Ok(archs
            .as_list()?
            .iter()
            .map(|arch| format!("-arch {arch}"))
            .collect::<Vec<_>>()
            .join(" "))
    })
    .map(Value::from)
}

fn sdkroot(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "sdk_version")?;
    global(interp, |interp| sdk_path(interp, argv[1].as_str())).map(Value::from)
}

fn cxx_stdlib(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    global(interp, |interp| {
        let standard = get(interp, "compiler.cxx_standard")?;
        let stdlib = optional(interp, "cxx_stdlib")?.unwrap_or_default();
        Ok(if standard.is_empty() {
            String::new()
        } else if stdlib == "libstdc++" && at_least(interp, "compiler.cxx_standard", 2011)? {
            "macports-libstdc++".into()
        } else {
            stdlib
        })
    })
    .map(Value::from)
}

fn should_add_stdlib(interp: &mut Interp) -> Result<bool, Exception> {
    let stdlib = get(interp, "configure.cxx_stdlib")?;
    if stdlib.is_empty() {
        return Ok(false);
    }
    let cxx = get(interp, "configure.cxx")?;
    if cxx.contains("clang") {
        return Ok(true);
    }
    let arch = get(interp, "configure.build_arch")?;
    let gcc_mp = Regex::new(r"g.*-mp-").unwrap().is_match(&cxx);
    if gcc_mp && !matches!(arch.as_str(), "ppc" | "ppc64") && stdlib != "macports-libstdc++" {
        let version = cxx.rsplit('-').next().unwrap_or("");
        return Ok(version == "devel" || version.parse::<f64>().is_ok_and(|v| v >= 10.0));
    }
    Ok(false)
}

fn should_add_cxx_abi(interp: &mut Interp) -> Result<bool, Exception> {
    let darwin = optional(interp, "os.platform")?.unwrap_or_default() == "darwin";
    Ok(os_major(interp)? < 13
        && darwin
        && get(interp, "configure.cxx_stdlib")? == "macports-libstdc++")
}

fn add_stdlib(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    global(interp, should_add_stdlib).map(Value::from)
}

fn add_cxx_abi(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    global(interp, should_add_cxx_abi).map(Value::from)
}

/// Call through the interpreter so PortGroups that wrap these helpers apply.
fn ask(interp: &mut Interp, command: &str) -> Result<bool, Exception> {
    interp
        .eval_value(&Value::from(vec![Value::from(command)]))?
        .as_bool()
}

fn cxxflags(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "flags")?;
    let mut flags = argv[1].as_list()?.to_vec();
    global(interp, |interp| {
        if ask(interp, "portconfigure::should_add_stdlib")? {
            flags.push(format!("-stdlib={}", get(interp, "configure.cxx_stdlib")?).into());
        }
        if ask(interp, "portconfigure::should_add_cxx_abi")? {
            flags.push("-D_GLIBCXX_USE_CXX11_ABI=0".into());
        }
        Ok(Value::from(flags))
    })
}

fn cppflags(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    global(interp, |interp| {
        Ok(if flag(interp, "compiler.limit_flags")? {
            String::new()
        } else {
            format!("-I{}/include", get(interp, "prefix")?)
        })
    })
    .map(Value::from)
}

fn ldflags(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    global(interp, |interp| {
        Ok(if flag(interp, "compiler.limit_flags")? {
            "-Wl,-headerpad_max_install_names".to_string()
        } else {
            format!(
                "-L{}/lib -Wl,-headerpad_max_install_names",
                get(interp, "prefix")?
            )
        })
    })
    .map(Value::from)
}

fn developer_tool(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "name")?;
    global(interp, |interp| {
        developer_tool_path(interp, argv[1].as_str())
    })
    .map(Value::from)
}

fn tools_version(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "compiler")?;
    global(interp, |interp| tools_version_of(interp, argv[1].as_str())).map(Value::from)
}

fn compiler_version(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "compiler developer_dir")?;
    compiler_build_version(argv[1].as_str(), argv[2].as_str()).map(Value::from)
}

fn tool_path(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "tool")?;
    Ok(find_tool(argv[1].as_str()).into())
}

/// Lines of the tree's per-SDK implicit function declaration whitelist.
fn whitelist(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "sdk_version")?;
    let Some(root) = interp
        .context::<State>(id)
        .groups
        .resource_directory
        .clone()
    else {
        return Ok(Value::empty());
    };
    let path = root.join(format!(
        "port1.0/checks/implicit_function_declaration/macosx{}.sdk.list",
        argv[1]
    ));
    let entries: Vec<Value> = std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(Value::from)
        .collect();
    Ok(Value::from(entries))
}
