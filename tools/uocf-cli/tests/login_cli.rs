//! `login show` and `login set` on a synthetic Classic folder.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const SHIPPED_LOGIN_CFG: &str = ";Loginservers for Ultima Online\r\nLoginServer=127.0.0.1,2593\r\n;LoginServer=107.23.85.115,7775\r\n";

fn temp_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("uocf-login-cli-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn uocf_cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_uocf-cli"))
}

#[test]
fn login_set_rewrites_the_server_lines_and_login_show_reads_them() {
    let dir = temp_dir("classic");
    let client = dir.join("client");
    fs::create_dir_all(&client).unwrap();
    fs::write(client.join("Login.cfg"), SHIPPED_LOGIN_CFG).unwrap();
    fs::write(client.join("uo.cfg"), "CacheSize=25\r\n").unwrap();
    let output = dir.join("login.cfg");
    let set = uocf_cli()
        .args(["login", "set"])
        .arg(&client)
        .arg("--output")
        .arg(&output)
        .args(["--server", "10.0.0.5:2593", "--server", "shard.example.org:2594"])
        .output()
        .unwrap();
    assert!(set.status.success(), "{}", String::from_utf8_lossy(&set.stderr));
    assert_eq!(
        fs::read_to_string(&output).unwrap(),
        ";Loginservers for Ultima Online\r\nLoginServer=10.0.0.5,2593\r\nLoginServer=shard.example.org,2594\r\n\
         ;LoginServer=107.23.85.115,7775\r\n"
    );
    assert_eq!(fs::read_to_string(client.join("Login.cfg")).unwrap(), SHIPPED_LOGIN_CFG, "the input is only read");

    let again = uocf_cli().args(["login", "set"]).arg(&client).arg("--output").arg(&output).args(["--server", "1.2.3.4:5"]).output().unwrap();
    assert!(!again.status.success(), "the output exists");
    let named = uocf_cli().args(["login", "set"]).arg(&client).arg("--output").arg(dir.join("x.cfg")).args(["--server", "a=1.2.3.4:5"]).output().unwrap();
    assert!(!named.status.success(), "login.cfg entries have no names");
    let bad = uocf_cli().args(["login", "set"]).arg(&client).arg("--output").arg(dir.join("y.cfg")).args(["--server", "1.2.3.400:5"]).output().unwrap();
    assert!(!bad.status.success(), "not an IPv4 address");

    let show = uocf_cli().args(["login", "show"]).arg(&client).output().unwrap();
    let text = String::from_utf8_lossy(&show.stdout);
    assert!(show.status.success(), "{}", String::from_utf8_lossy(&show.stderr));
    assert!(text.contains("family    Classic") && text.contains("127.0.0.1:2593"), "{text}");
    let _ = fs::remove_dir_all(&dir);
}
