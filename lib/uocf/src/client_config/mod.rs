//! Client configuration files a client folder holds next to its executable: the login server lists
//! ([`login_servers`]: Classic `login.cfg` and `uo.cfg`, Enhanced Client and Kingdom Reborn `LoginServers*.xml`).
//! Files are only read; writers take an explicit output path. Where each client family reads its login server from,
//! with the evidence: `LOGIN_SERVER.md` next to this module.

pub mod login_servers;
