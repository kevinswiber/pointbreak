//! HTTP topology contract for the server-laid supersession-DAG geometry.
//!
//! The layout is computed server-side and emitted as the additive `laidOut`
//! geometry on each `/api/objects` thread. These tests assert the layout's
//! TOPOLOGY over a real fork — node set, edge `from`/`to`, head/superseded
//! status, peer-equal head rank, normalized origin — and NEVER exact pixel
//! coordinates (those are a property of the pinned engine version, not a stable
//! contract).

mod support;

use support::git_repo::GitRepo;
use support::inspect::{Inspector, capture_supersession_round};

/// Build the canonical fork: A (root), B supersedes A, C supersedes A -> heads
/// {B,C}. Returns the `/api/objects` payload plus the three revision ids.
fn build_fork() -> (serde_json::Value, String, String, String) {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let a = capture_supersession_round(repo.path(), None);
    let b = capture_supersession_round(repo.path(), Some(&a));
    let c = capture_supersession_round(repo.path(), Some(&a));
    assert_ne!(b, c, "the two successors must be distinct");
    let objects = Inspector::spawn(repo.path()).get_json("/api/objects");
    (objects, a, b, c)
}

fn forked_objects() -> serde_json::Value {
    build_fork().0
}

#[test]
fn laid_out_dag_places_competing_heads_as_equal_peers() {
    let objects = forked_objects();
    let thread = &objects["threads"][0];
    assert_eq!(thread["competing"], true);

    let laid = &thread["laidOut"];
    let nodes = laid["nodes"].as_array().expect("laidOut.nodes");
    assert_eq!(nodes.len(), 3, "three revisions in the fork");

    // Bounds present and positive (a real layout, not a stub). NEVER exact pixels.
    let (bw, bh) = (
        laid["bounds"]["w"].as_f64().unwrap(),
        laid["bounds"]["h"].as_f64().unwrap(),
    );
    assert!(bw > 0.0 && bh > 0.0);

    // Normalized origin: every node box lies within `viewBox="0 0 w h"`, so the
    // client never clips. (Containment over the emitted axis, not exact pixels.)
    for n in nodes {
        let (x, y, w, h) = (
            n["x"].as_f64().unwrap(),
            n["y"].as_f64().unwrap(),
            n["w"].as_f64().unwrap(),
            n["h"].as_f64().unwrap(),
        );
        assert!(
            x - w / 2.0 >= -0.5 && x + w / 2.0 <= bw + 0.5,
            "node fits the viewBox width"
        );
        assert!(
            y - h / 2.0 >= -0.5 && y + h / 2.0 <= bh + 0.5,
            "node fits the viewBox height"
        );
    }

    // Topology: exactly two heads, exactly one superseded node.
    let heads: Vec<&serde_json::Value> = nodes.iter().filter(|n| n["isHead"] == true).collect();
    let superseded: Vec<&serde_json::Value> =
        nodes.iter().filter(|n| n["isSuperseded"] == true).collect();
    assert_eq!(heads.len(), 2, "two competing heads");
    assert_eq!(superseded.len(), 1, "one superseded root");

    // Two edges, each B->A / C->A: `from` supersedes `to`, `to` is the superseded root.
    let edges = laid["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2);
    let root_id = superseded[0]["id"].as_str().unwrap();
    for e in edges {
        assert_eq!(e["to"], root_id, "every edge points at the superseded root");
        assert!(e["from"] != root_id, "the root supersedes nothing");
        assert!(
            e["path"].as_array().unwrap().len() >= 2,
            "a routed polyline has >=2 points"
        );
    }

    // No-trunk / peer-equal heads, asserted as topology: both heads sit at the
    // same rank (equal y), and NO non-head node shares or precedes that rank.
    let head_ys: Vec<f64> = heads.iter().map(|n| n["y"].as_f64().unwrap()).collect();
    assert!(
        (head_ys[0] - head_ys[1]).abs() < 1.0,
        "competing heads share a rank (equal y)"
    );
    let min_head_y = head_ys.iter().cloned().fold(f64::INFINITY, f64::min);
    for n in nodes {
        if n["isHead"] != true {
            assert!(
                n["y"].as_f64().unwrap() >= min_head_y,
                "no non-head node sits above a head"
            );
        }
    }
}

#[test]
fn forked_thread_payload_surfaces_competing_heads_and_peer_layout() {
    // The supersession contract and the laidOut peer-equality topology together,
    // over the fork the live store has none of — the synthetic-fixture state.
    let (objects, a, b, c) = build_fork();
    assert_eq!(objects["threadCount"], 1);
    let thread = &objects["threads"][0];

    // Competing heads {B,C}, root superseded by BOTH.
    assert_eq!(thread["competing"], true);
    let heads: Vec<&str> = thread["heads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h.as_str().unwrap())
        .collect();
    assert_eq!(heads.len(), 2);
    assert!(heads.contains(&b.as_str()) && heads.contains(&c.as_str()));
    let superseders: Vec<&str> = objects["supersededBy"][&a]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert!(superseders.contains(&b.as_str()) && superseders.contains(&c.as_str()));

    // The laidOut peer-equality topology: three nodes, two equal-rank heads, one
    // superseded root, two edges from the heads to the root, no node above a head.
    let laid = &thread["laidOut"];
    let nodes = laid["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 3);
    let head_nodes: Vec<&serde_json::Value> =
        nodes.iter().filter(|n| n["isHead"] == true).collect();
    assert_eq!(head_nodes.len(), 2, "two competing head nodes");
    let head_ys: Vec<f64> = head_nodes
        .iter()
        .map(|n| n["y"].as_f64().unwrap())
        .collect();
    assert!(
        (head_ys[0] - head_ys[1]).abs() < 1.0,
        "competing heads share a rank"
    );
    let min_head_y = head_ys.iter().cloned().fold(f64::INFINITY, f64::min);
    for n in nodes {
        if n["isHead"] != true {
            assert!(
                n["y"].as_f64().unwrap() >= min_head_y,
                "no non-head node sits above a head"
            );
        }
    }
    let edges = laid["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2);
    let root_id = nodes.iter().find(|n| n["isSuperseded"] == true).unwrap()["id"]
        .as_str()
        .unwrap();
    for e in edges {
        assert_eq!(
            e["to"], root_id,
            "`from` supersedes `to` = the superseded root"
        );
    }
}

#[test]
fn laid_out_dag_degenerate_single_node_thread_has_no_edges() {
    let repo = GitRepo::new();
    repo.write("src/lib.rs", "pub fn value() -> u32 { 1 }\n");
    repo.commit_all("base");
    repo.write("src/lib.rs", "pub fn value() -> u32 { 2 }\n");
    let _root = capture_supersession_round(repo.path(), None);

    let objects = Inspector::spawn(repo.path()).get_json("/api/objects");
    let laid = &objects["threads"][0]["laidOut"];
    assert_eq!(laid["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(laid["edges"].as_array().unwrap().len(), 0);
    assert_eq!(laid["nodes"][0]["isHead"], true);
}
