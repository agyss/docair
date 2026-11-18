//! Test results on the graph: Playwright JSON reports bound to `test` nodes (SPEC §12.3).

use std::path::Path;

use serde_json::Value;

use crate::model::{Diagnostic, Node, TestResult};

/// One test as a report states it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Reported {
    file: String,
    title: String,
    result: TestResult,
}

/***
 * Read the test reports named in `[tests] results` and put each result on its test node.
 *
 * A result binds to the `test` node in the same file whose name is the
 * test's title. Results without a node, and unreadable reports, are W111,
 * and so is a report outside the project (`../x`, `/dev/zero`, a link out).
 *
 * @id     docair-core.test_results.attach
 * @flow   scan 11 "test results are put on the test nodes"
 * @reads  test-reports -> Playwright JSON
 ***/
pub fn attach(root: &Path, reports: &[String], nodes: &mut [Node], diags: &mut Vec<Diagnostic>) {
    for rel in reports {
        if !root.join(rel).exists() {
            // No run yet: the tests show without results.
            continue;
        }
        let Some(path) = crate::config::inside(root, rel).filter(|p| p.is_file()) else {
            diags.push(
                Diagnostic::new(
                    "W111",
                    format!("the test report {rel} is no file inside the project"),
                )
                .at("docair.toml", 1),
            );
            continue;
        };
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
        let report = match parsed {
            Ok(v) => v,
            Err(e) => {
                diags.push(
                    Diagnostic::new("W111", format!("cannot read the test report: {e}")).at(rel, 1),
                );
                continue;
            }
        };
        let mut found = Vec::new();
        for suite in report["suites"].as_array().into_iter().flatten() {
            collect(suite, &mut found);
        }
        for r in found {
            let node = nodes.iter_mut().find(|n| {
                n.kind == "test"
                    && n.name == r.title
                    && n.source
                        .as_ref()
                        .is_some_and(|s| same_file(&s.file, &r.file))
            });
            match node {
                Some(n) => n.test = Some(r.result),
                None => diags.push(
                    Diagnostic::new(
                        "W111",
                        format!("test `{}` in {} has no `test` node", r.title, r.file),
                    )
                    .at(rel, 1),
                ),
            }
        }
    }
}

/// Report paths are relative to the test folder; node paths to the project root.
fn same_file(node_file: &str, report_file: &str) -> bool {
    let report_file = report_file.trim_start_matches("./");
    node_file == report_file || node_file.ends_with(&format!("/{report_file}"))
}

fn collect(suite: &Value, out: &mut Vec<Reported>) {
    for spec in suite["specs"].as_array().into_iter().flatten() {
        let file = spec["file"]
            .as_str()
            .or_else(|| suite["file"].as_str())
            .unwrap_or_default();
        let mut statuses = Vec::new();
        let mut duration = 0;
        let mut at: Option<String> = None;
        for test in spec["tests"].as_array().into_iter().flatten() {
            statuses.push(test["status"].as_str().unwrap_or("skipped").to_string());
            if let Some(last) = test["results"].as_array().and_then(|r| r.last()) {
                duration += last["duration"].as_u64().unwrap_or(0);
                if let Some(start) = last["startTime"].as_str()
                    && at.as_deref().is_none_or(|a| start < a)
                {
                    at = Some(start.to_string());
                }
            }
        }
        // Several projects can run the same test: the worst outcome wins.
        let status = if statuses.iter().any(|s| s == "unexpected") {
            "failed"
        } else if statuses.iter().any(|s| s == "flaky") {
            "flaky"
        } else if !statuses.is_empty() && statuses.iter().all(|s| s == "skipped") {
            "skipped"
        } else if statuses.is_empty() {
            continue;
        } else {
            "passed"
        };
        out.push(Reported {
            file: file.to_string(),
            title: spec["title"].as_str().unwrap_or_default().to_string(),
            result: TestResult {
                status: status.to_string(),
                duration_ms: duration,
                at,
            },
        });
    }
    for child in suite["suites"].as_array().into_iter().flatten() {
        collect(child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Origin, SourceRef};

    fn test_node(name: &str, file: &str) -> Node {
        Node {
            id: format!("t.{name}"),
            kind: "test".into(),
            name: name.into(),
            summary: String::new(),
            description: String::new(),
            part_of: None,
            source: Some(SourceRef {
                file: file.into(),
                line: 1,
            }),
            symbol: None,
            http: None,
            deployed_as: None,
            docs: vec![],
            tags: vec![],
            origin: Origin::Authored,
            test: None,
            signature: None,
            runs_in: None,
            code_from: None,
        }
    }

    #[test]
    fn playwright_results_bind_by_file_and_title() {
        let report = serde_json::json!({
            "suites": [{
                "title": "web.spec.ts", "file": "web.spec.ts",
                "specs": [
                    { "title": "a page", "file": "web.spec.ts", "tests": [
                        { "status": "expected", "results": [{ "status": "passed", "duration": 1200, "startTime": "2026-11-15T08:00:00.000Z" }] },
                        { "status": "flaky", "results": [{ "status": "passed", "duration": 300, "startTime": "2026-11-15T08:00:02.000Z" }] }
                    ]}
                ],
                "suites": [{ "title": "chat", "specs": [
                    { "title": "a follow-up", "file": "web.spec.ts", "tests": [{ "status": "unexpected", "results": [{ "status": "failed", "duration": 50 }] }] },
                    { "title": "ghost", "file": "web.spec.ts", "tests": [{ "status": "expected", "results": [] }] }
                ]}]
            }]
        });
        let dir = std::env::temp_dir().join(format!("docair-tests-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("r.json"), report.to_string()).unwrap();
        let mut nodes = vec![
            test_node("a page", "e2e/tests/web.spec.ts"),
            test_node("a follow-up", "e2e/tests/web.spec.ts"),
            test_node("a page", "e2e/tests/other.spec.ts"),
        ];
        let mut diags = vec![];
        attach(
            &dir,
            &["r.json".into(), "missing.json".into()],
            &mut nodes,
            &mut diags,
        );
        let a = nodes[0].test.as_ref().unwrap();
        assert_eq!((a.status.as_str(), a.duration_ms), ("flaky", 1500));
        assert_eq!(a.at.as_deref(), Some("2026-11-15T08:00:00.000Z"));
        assert_eq!(nodes[1].test.as_ref().unwrap().status, "failed");
        assert!(nodes[2].test.is_none(), "same title, other file");
        assert_eq!(diags.len(), 1, "ghost has no node: {diags:?}");
        assert_eq!(diags[0].code, "W111");
        std::fs::remove_dir_all(&dir).ok();
    }
}
