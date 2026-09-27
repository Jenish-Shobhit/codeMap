//! Performance checks, ignored by default (timings vary with load):
//! `cargo test --release --test perf -- --ignored --nocapture`

mod common;

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;

use codemap::app::{App, Options, View};
use codemap::git;
use common::*;

/// 2,000 commits with a branch merged every 50, written with fast-import.
fn big_history(dir: &std::path::Path) {
    git(dir, &["init", "-q"]);
    let mut s = String::new();
    let mut mark = 0;
    let mut main_tip = 0;
    for i in 0..2000 {
        mark += 1;
        let content = format!("line {i}\n");
        let parent = if main_tip > 0 {
            format!("from :{main_tip}\n")
        } else {
            String::new()
        };
        s.push_str(&format!(
            "commit refs/heads/main\nmark :{mark}\ncommitter T <t@t> {} +0000\ndata {}\ncommit {i}\n{parent}M 100644 inline file{}.txt\ndata {}\n{content}\n",
            1_700_000_000 + i,
            format!("commit {i}").len(),
            i % 50,
            content.len()
        ));
        main_tip = mark;
        if i % 50 == 49 {
            // a side branch of two commits merged back
            mark += 1;
            let b1 = mark;
            s.push_str(&format!(
                "commit refs/heads/side{i}\nmark :{b1}\ncommitter T <t@t> {} +0000\ndata 4\nside\nfrom :{main_tip}\nM 100644 inline side{i}.txt\ndata 2\ns\n\n",
                1_700_000_000 + i
            ));
            mark += 1;
            s.push_str(&format!(
                "commit refs/heads/main\nmark :{mark}\ncommitter T <t@t> {} +0000\ndata 5\nmerge\nfrom :{main_tip}\nmerge :{b1}\n\n",
                1_700_000_000 + i
            ));
            main_tip = mark;
        }
    }
    let mut child = Command::new("git")
        .current_dir(dir)
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(s.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    git(dir, &["reset", "-q", "--hard", "main"]);
}

#[test]
#[ignore]
fn history_of_2000_commits_renders_within_150ms() {
    let t = TempDir::new("perf-history");
    big_history(t.path());
    let state = TempDir::new("perf-history-state");
    let t0 = Instant::now();
    let commits = git::graph::log(t.path(), 2000).unwrap();
    let t_log = t0.elapsed();
    let rows = git::graph::layout(&commits);
    let t_layout = t0.elapsed();
    assert_eq!(commits.len(), 2000);
    let mut app = App::new(Options {
        path: Some(t.path().to_path_buf()),
        view: View::History,
        context: None,
        client: None,
        store: codemap::store::Store::new(state.path()),
        theme: dracula(),
    });
    app.load_blocking();
    let t1 = Instant::now();
    let _ = codemap::ui::render_text(&mut app, 180, 50);
    let t_render = t1.elapsed();
    println!(
        "perf: git log 2000 commits {:.1} ms, lanes {:.1} ms ({} rows), frame {:.1} ms; log+lanes+frame {:.1} ms",
        t_log.as_secs_f64() * 1000.0,
        (t_layout - t_log).as_secs_f64() * 1000.0,
        rows.len(),
        t_render.as_secs_f64() * 1000.0,
        (t_layout + t_render).as_secs_f64() * 1000.0
    );
    assert!(t_layout + t_render < std::time::Duration::from_millis(150));
}

#[test]
#[ignore]
fn huge_repo_paints_first_and_parses_lazily() {
    let t = TempDir::new("perf-huge");
    let p = t.path();
    git(p, &["init", "-q"]);
    for d in 0..300 {
        for f in 0..40 {
            write(
                p,
                &format!("pkg{d:03}/mod{f:02}.py"),
                &format!("def f{f}(x):\n    if x:\n        return g{f}(x)\n    return 0\n\ndef g{f}(x):\n    return x\n"),
            );
        }
    }
    git(p, &["add", "-A"]);
    git(p, &["commit", "-q", "-m", "12k files"]);
    let state = TempDir::new("perf-huge-state");
    let mk = || {
        App::new(Options {
            path: Some(p.to_path_buf()),
            view: View::Map,
            context: None,
            client: None,
            store: codemap::store::Store::new(state.path()),
            theme: dracula(),
        })
    };
    // First paint: nothing loaded yet.
    let t0 = Instant::now();
    let mut app = mk();
    let _ = codemap::ui::render_text(&mut app, 180, 50);
    let first = t0.elapsed();
    // Full load (listing, folder-level parse, diff, history) synchronously.
    let t1 = Instant::now();
    let mut app = mk();
    app.load_blocking();
    let screen = codemap::ui::render_text(&mut app, 180, 50);
    app.drain();
    let loaded = t1.elapsed();
    assert!(screen.contains("page 1 of"), "{screen}");
    assert!(
        app.index.parsed_count() < 100,
        "parsed {} files up front",
        app.index.parsed_count()
    );
    // Zooming into a folder parses just that folder.
    let t2 = Instant::now();
    app.zoom_to(codemap::map::Level::Dir("pkg150".into()), None);
    let _ = codemap::ui::render_text(&mut app, 180, 50);
    app.drain();
    let screen = codemap::ui::render_text(&mut app, 180, 50);
    let zoom = t2.elapsed();
    assert!(screen.contains("mod00.py"), "{screen}");
    assert!(
        screen.contains("f0"),
        "the folder's functions are parsed on zoom\n{screen}"
    );
    assert_eq!(app.index.parsed_count(), 40);
    println!(
        "perf: 12,000 files: first paint {:.1} ms, loaded {:.0} ms (parsed {} files), zoom into a folder {:.0} ms",
        first.as_secs_f64() * 1000.0,
        loaded.as_secs_f64() * 1000.0,
        app.index.parsed_count(),
        zoom.as_secs_f64() * 1000.0
    );
}
