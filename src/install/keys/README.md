The bundled `macports-archives-2025.pub` is the MacPorts archive-signing public
key, retrieved from the official macports-base repository on 2026-10-30:
https://github.com/macports/macports-base/blob/master/keys/archives/macports-archives-2025.pub

It is intentionally distinct from the base-update and ports-tree signing keys.
The key is pinned in the executable. Archive mirrors cannot supply or replace
trust roots. Public-key comments are informational and unauthenticated.

`macports-ports-2025.pub` is the MacPorts ports-tree signing public key from the
same repository (`keys/ports/macports-ports-2025.pub`), copied on 2026-10-31
from an installed MacPorts 2.12.7. `port sync` uses it to verify `ports.tar.gz`
and the platform PortIndex, together with any `keys/ports/*.pub` installed in
the configured prefix.
