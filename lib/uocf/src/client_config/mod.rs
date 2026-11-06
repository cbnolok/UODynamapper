//! Login-server configuration inspection and file editing ([`login_servers`]). Classic reads `uo.cfg` and its
//! login file. In the traced Enhanced Client and Kingdom Reborn builds, `LoginServers*.xml` has no effect:
//! inspection reports the executable's built-in addresses and lists XML entries as unused.
//! Writers take an explicit output path. See `LOGIN_SERVER.md` next to this module for the evidence.

pub mod login_servers;
