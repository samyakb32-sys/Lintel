// Proves the developer alerts without the app (read-only, silent):
//
//   cargo run -p meteor --example alerts-probe            the rules, over made-up polls
//   cargo run -p meteor --example alerts-probe -- --gh    also lists the owner's open pull requests
//                                                         through the local `gh` login (counts only)
//
// The rules are the app's own file alerts.rs, compiled here as it is.

#![allow(dead_code)]

#[path = "../src/alerts.rs"]
mod alerts;

use alerts::*;

fn dep(id: &str, project: &str, state: &str, created: u64) -> Dep {
    Dep { id: id.into(), project: project.into(), state: state.into(), created_ms: created, host: format!("{project}-{id}.vercel.app"), detail: "main".into() }
}

fn main() {
    let started = 1_000_000;
    let mut seen: VercelSeen = None;
    let mut dedupe = Dedupe::default();
    let none: Vec<String> = vec![];

    // Poll 1: one deploy done, one building, at launch. Silent.
    let p1 = [dep("a", "site", "READY", 10), dep("b", "site", "BUILDING", 20)];
    assert!(vercel_alerts(&mut seen, &p1, &none, started, &mut dedupe).is_empty());
    println!("1. first poll fills silently: 0 alerts for {} deploys on screen at launch", p1.len());

    // Poll 2: b turns READY, c is new and already ERROR (built between polls), d is old and ERROR but listed for the first time.
    let p2 = [dep("a", "site", "READY", 10), dep("b", "site", "READY", 20), dep("c", "api", "ERROR", started + 5), dep("d", "api", "ERROR", 5)];
    let out = vercel_alerts(&mut seen, &p2, &none, started, &mut dedupe);
    let kinds: Vec<_> = out.iter().map(|a| (a.subject.as_str(), a.kind)).collect();
    assert_eq!(kinds, vec![("site", "deployReady"), ("api", "deployFailed")]);
    println!("2. new since start: {kinds:?} (the old failed one 'd' and the unchanged 'a' stay quiet)");

    // Poll 3: nothing changes -> nothing. Poll 4: a rebuild of the same id never repeats.
    assert!(vercel_alerts(&mut seen, &p2, &none, started, &mut dedupe).is_empty());
    let mut again = out.clone();
    again.retain(|a| dedupe.first(&a.key));
    assert!(again.is_empty());
    println!("3. same run twice: 0 alerts (poll unchanged, and the once-only key refuses a repeat)");

    // Watch list: only the watched project is followed.
    let mut seen2: VercelSeen = None;
    let mut d2 = Dedupe::default();
    let watch = vec!["API".to_string()];
    vercel_alerts(&mut seen2, &[dep("x", "site", "BUILDING", started + 1), dep("y", "api", "BUILDING", started + 1)], &watch, started, &mut d2);
    let out = vercel_alerts(&mut seen2, &[dep("x", "site", "ERROR", started + 1), dep("y", "api", "READY", started + 1)], &watch, started, &mut d2);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].subject, "api");
    println!("4. watch list [API]: 1 alert, for 'api' only; 'site' ignored");

    // Watch list parsing: bad names, duplicates, the 20 cap.
    let raw: serde_json::Value = serde_json::json!(["a/b", "A/B", "bad name/x", "o/n\"x", "../x", "c/d", 5]);
    assert_eq!(parse_watch(Some(&raw), repo_ok), vec!["a/b".to_string(), "c/d".to_string()]);
    let many: Vec<String> = (0..30).map(|i| format!("o/r{i}")).collect();
    assert_eq!(parse_watch(Some(&serde_json::json!(many)), repo_ok).len(), 20);
    assert!(watched(&[], "any/thing") && watched(&["O/R".into()], "o/r") && !watched(&["o/r".into()], "o/q"));
    println!("5. watch list: bad/duplicate names dropped, capped at 20, empty list watches everything");

    // Back-off: errors double the wait, a limit waits for the reset, a good answer resets the count.
    let mut b = Backoff::default();
    let now = 5_000_000;
    assert!(!b.blocked(now));
    b.fail(now, 30);
    assert!(b.blocked(now + 29_000) && !b.blocked(now + 31_000));
    b.fail(now + 31_000, 30);
    assert!(b.blocked(now + 31_000 + 59_000) && !b.blocked(now + 31_000 + 61_000));
    b.ok();
    let mut l = Backoff::default();
    l.limited(now, Some(now / 1000 + 600), None);
    assert!(l.blocked(now + 500_000) && !l.blocked(now + 602_000));
    let mut r = Backoff::default();
    r.limited(now, None, Some(7));
    assert!(r.blocked(now + 6_000) && !r.blocked(now + 7_100));
    let mut c = Backoff::default();
    for i in 0..30 {
        c.fail(now, 30 * (i + 1) / (i + 1));
    }
    assert!(c.blocked(now + 1_799_000) && !c.blocked(now + 1_801_000));
    println!("6. back-off: 30 s then 60 s on errors, 30 min cap, rate limit waits for its reset or Retry-After");

    if std::env::args().any(|a| a == "--gh") {
        gh_prs();
    }
    println!("\nall checks passed");
}

/// One read-only GitHub search through the local `gh` login. Counts only.
fn gh_prs() {
    let out = std::process::Command::new("gh")
        .args(["api", "search/issues?q=is:pr+is:open+author:@me&per_page=1", "--jq", ".total_count"])
        .output();
    match out {
        Ok(o) if o.status.success() => println!("7. gh login: {} open pull requests of mine (read-only)", String::from_utf8_lossy(&o.stdout).trim()),
        Ok(o) => println!("7. gh login: refused ({})", o.status),
        Err(_) => println!("7. gh login: gh not found"),
    }
    let out = std::process::Command::new("gh")
        .args(["api", "search/issues?q=is:pr+is:open+review-requested:@me&per_page=1", "--jq", ".total_count"])
        .output();
    if let Ok(o) = out.as_ref() {
        if o.status.success() {
            println!("   review requested from me: {}", String::from_utf8_lossy(&o.stdout).trim());
        }
    }
}
