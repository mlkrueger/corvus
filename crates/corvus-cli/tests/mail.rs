//! Plan 0012 step 9 through the real binary: the "not reachable" warning and `corvus mail reassign`.
#![allow(clippy::unwrap_used)] // test helpers outside #[test] fns

mod common;

use common::{Home, json_stdout, stderr};
use serde_json::{Value, json};

fn ok(home: &Home, args: &[&str]) -> Value {
    json_stdout(&home.corvus().args(args).output().unwrap())
}

/// Registers `agent` and ends the session: known to Corvus, with nothing that can deliver to it.
fn offline(home: &Home, agent: &str) {
    let session = ok(home, &["session", "register", "--agent", agent]);
    ok(home, &["session", "end", session["id"].as_str().unwrap()]);
}

#[test]
fn tell_warns_when_the_recipient_is_not_reachable_and_still_delivers() {
    let home = Home::new();
    offline(&home, "old");
    ok(&home, &["session", "register", "--agent", "live"]);

    let out = home
        .corvus()
        .args(["tell", "old", "are you there?"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let sent = json_stdout(&out);
    assert_eq!(sent["recipient"], "old");
    let warning = stderr(&out);
    assert!(warning.contains("warning: old is not reachable"), "{warning}");
    assert!(warning.contains("corvus mail reassign old <to>"), "{warning}");
    assert!(!warning.contains("are you there?"), "no message text on stderr");
    assert_eq!(ok(&home, &["inbox", "read", "old"])[0]["id"], sent["id"]);

    // An agent with an open session gets no warning.
    let out = home.corvus().args(["tell", "live", "hello"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(stderr(&out), "");
}

#[test]
fn tell_does_not_warn_while_a_watcher_holds_the_agents_lock() {
    let home = Home::new();
    offline(&home, "old");
    let lock = home.state_dir().join("watch").join("old").join("lock");
    std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
        .unwrap();
    file.try_lock().unwrap();
    let out = home.corvus().args(["tell", "old", "ping"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(stderr(&out), "");
    drop(file);
    let out = home.corvus().args(["tell", "old", "ping"]).output().unwrap();
    assert!(stderr(&out).contains("not reachable"));
}

#[test]
fn mail_reassign_moves_unread_direct_mail_once() {
    let home = Home::new();
    offline(&home, "old");
    ok(&home, &["session", "register", "--agent", "new"]);
    let first = ok(&home, &["tell", "old", "first", "--type", "question"]);
    let second = ok(&home, &["tell", "old", "second"]);

    let moved = ok(&home, &["mail", "reassign", "old", "new"]);
    assert_eq!(moved["from"], "old");
    assert_eq!(moved["to"], "new");
    let pairs = moved["moved"].as_array().unwrap();
    assert_eq!(pairs.len(), 2);
    assert_eq!(pairs[0]["original_id"], first["id"]);
    assert_eq!(pairs[1]["original_id"], second["id"]);

    let inbox = ok(&home, &["inbox", "read", "new"]);
    assert_eq!(inbox.as_array().unwrap().len(), 2);
    assert_eq!(inbox[0]["id"], pairs[0]["message_id"]);
    assert_eq!(inbox[0]["reassigned_from"], first["id"]);
    assert_eq!(inbox[0]["body"], "first");
    assert_eq!(inbox[0]["type"], "question");
    assert_eq!(inbox[0]["sender"], "operator");
    assert_eq!(ok(&home, &["inbox", "read", "old"]), json!([]));

    // Again: nothing left to move.
    assert_eq!(ok(&home, &["mail", "reassign", "old", "new"])["moved"], json!([]));

    let same = home
        .corvus()
        .args(["mail", "reassign", "old", "old"])
        .output()
        .unwrap();
    assert!(!same.status.success());
    assert!(stderr(&same).contains("validation"), "{}", stderr(&same));
    let ghost = home
        .corvus()
        .args(["mail", "reassign", "old", "ghost"])
        .output()
        .unwrap();
    assert!(!ghost.status.success());
    assert!(stderr(&ghost).contains("not_found"), "{}", stderr(&ghost));
}
