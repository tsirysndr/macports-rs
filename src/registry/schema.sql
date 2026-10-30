-- MacPorts registry schema 1.215, matching src/cregistry/sql.c in macports-base.
-- Created only through Writer::create; read-only queries never initialize it.
BEGIN;
CREATE TABLE metadata (key UNIQUE, value);
INSERT INTO metadata VALUES ('version', '1.215');
INSERT INTO metadata VALUES ('created', strftime('%s', 'now'));
CREATE TABLE ports (
 id INTEGER PRIMARY KEY, name TEXT COLLATE NOCASE, portfile TEXT, location TEXT,
 epoch INTEGER, version TEXT COLLATE VERSION, revision INTEGER, variants TEXT,
 requested_variants TEXT, state TEXT, date DATETIME, installtype TEXT, archs TEXT,
 requested INTEGER, os_platform TEXT, os_major INTEGER, cxx_stdlib TEXT,
 cxx_stdlib_overridden INTEGER, UNIQUE(name, epoch, version, revision, variants)
);
CREATE INDEX port_name ON ports(name, epoch, version, revision, variants);
CREATE INDEX port_state ON ports(state);
CREATE TABLE files (id INTEGER, path TEXT, actual_path TEXT, active INTEGER, binary BOOL,
 FOREIGN KEY(id) REFERENCES ports(id) ON DELETE CASCADE);
CREATE INDEX file_port ON files(id);
CREATE INDEX file_path ON files(path);
CREATE INDEX file_actual ON files(actual_path);
CREATE INDEX file_actual_nocase ON files(actual_path COLLATE NOCASE);
CREATE TABLE dependencies (id INTEGER, name TEXT, variants TEXT,
 FOREIGN KEY(id) REFERENCES ports(id) ON DELETE CASCADE);
CREATE INDEX dep_id ON dependencies(id);
CREATE INDEX dep_name ON dependencies(name);
CREATE TABLE portgroups (id INTEGER, name TEXT, version TEXT COLLATE VERSION,
 size INTEGER, sha256 TEXT, FOREIGN KEY(id) REFERENCES ports(id) ON DELETE CASCADE);
CREATE INDEX portgroup_id ON portgroups(id);
CREATE INDEX portgroup_open ON portgroups(id, name, version, size, sha256);
CREATE TABLE snapshots (id INTEGER PRIMARY KEY, created_at DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL, note TEXT);
CREATE TABLE snapshot_ports (id INTEGER PRIMARY KEY, snapshots_id INTEGER,
 port_name TEXT COLLATE NOCASE, requested INTEGER, state TEXT COLLATE NOCASE,
 variants TEXT, requested_variants TEXT,
 FOREIGN KEY(snapshots_id) REFERENCES snapshots(id) ON DELETE CASCADE);
CREATE TABLE snapshot_files (id INTEGER, path TEXT,
 FOREIGN KEY(id) REFERENCES snapshot_ports(id) ON DELETE CASCADE);
CREATE INDEX snapshot_file_path ON snapshot_files(path);
CREATE INDEX snapshot_file_id ON snapshot_files(id);
COMMIT;
