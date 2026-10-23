//! The menu, built from forge's `GET /api/stations` (the pipeline's `console.py list`). Platform-independent and
//! tested; the Windows tray in `main.rs` only draws it.

use serde_json::Value;

/// David's ruling (2026-10-23) for the workbench page, kept for the tray: everything shows except these.
pub const EXCLUDED: &[&str] = &["060_OPENAI_STATIONS", "60_OPENAI_STATIONS"];
/// Stations with no family (000, 041, 042, 047-049 ...) are grouped here so nothing is hidden.
pub const FRONT: &str = "Front stations";

#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub name: String,
    /// A `.bat` the feed listed for this station: the only thing the tray ever runs.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Station {
    pub label: String,
    pub folder: String,
    pub buttons: Vec<Button>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    pub stations: Vec<Station>,
}

/// `10_FIS` -> `10 · FIS`, `02_C_P_E_L` -> `02 · C P E L`, "" -> "Front stations".
pub fn group_title(family: &str) -> String {
    match family.split_once('_') {
        _ if family.is_empty() => FRONT.to_string(),
        Some(("", rest)) => {
            // `_system` -> `System`
            let mut c = rest.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        }
        Some((num, rest)) => format!("{num} · {}", rest.replace('_', " ")),
        None => family.to_string(),
    }
}

/// Group the feed by family, in family order, front stations last. Accepts `{"stations": [...]}` or a bare list.
pub fn build(feed: &Value) -> Vec<Group> {
    let rows = feed
        .get("stations")
        .unwrap_or(feed)
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut groups: Vec<(String, Group)> = Vec::new();
    for row in rows {
        let label = row["label"].as_str().unwrap_or_default().to_string();
        let path = row["path"].as_str().unwrap_or_default();
        if label.is_empty() || EXCLUDED.iter().any(|x| *x == label || path.ends_with(x)) {
            continue;
        }
        let family = row["family"].as_str().unwrap_or_default().to_string();
        let buttons = row["buttons"]
            .as_array()
            .map(|bs| {
                bs.iter()
                    .filter_map(|b| {
                        let p = b["path"].as_str()?;
                        p.to_ascii_lowercase().ends_with(".bat").then(|| Button {
                            name: b["name"].as_str().unwrap_or(p).to_string(),
                            path: p.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let station = Station {
            label: path
                .rsplit(['\\', '/'])
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or(&label)
                .to_string(),
            folder: row["front"].as_str().unwrap_or_default().to_string(),
            buttons,
        };
        match groups.iter_mut().find(|(f, _)| *f == family) {
            Some((_, g)) => g.stations.push(station),
            None => groups.push((
                family.clone(),
                Group {
                    name: group_title(&family),
                    stations: vec![station],
                },
            )),
        }
    }
    // Families in number order; the unnamed (front) group last.
    groups.sort_by(|(a, _), (b, _)| (a.is_empty(), a.as_str()).cmp(&(b.is_empty(), b.as_str())));
    for (_, g) in groups.iter_mut() {
        g.stations.sort_by(|a, b| a.label.cmp(&b.label));
    }
    groups.into_iter().map(|(_, g)| g).collect()
}

/// The menu as text: `forge-tray --print`, and what the tests read.
pub fn render(groups: &[Group]) -> String {
    let mut out = String::new();
    for g in groups {
        out.push_str(&format!("{}\n", g.name));
        for s in &g.stations {
            out.push_str(&format!("  {}\n", s.label));
            for b in &s.buttons {
                out.push_str(&format!("    {}\n", b.name));
            }
        }
    }
    out
}

pub fn forge_url() -> String {
    std::env::var("FORGE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8790".into())
        .trim_end_matches('/')
        .to_string()
}

/// Fetch the feed from forge (plain HTTP on the local machine; forge listens on 127.0.0.1 only). The error says what
/// to do; the tray shows it instead of an empty or typed-in menu.
pub fn fetch(base: &str) -> anyhow::Result<Value> {
    let (status, body) = http_get(base, "/api/stations").map_err(|e| {
        anyhow::anyhow!("forge not reachable at {base} ({e}); start it with `forge serve`")
    })?;
    let json: Value = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("{base}/api/stations: not JSON ({e})"))?;
    if status != 200 {
        anyhow::bail!(
            "{base}/api/stations: {}",
            json["error"].as_str().unwrap_or("error")
        );
    }
    Ok(json)
}

/// Minimal HTTP/1.1 GET for `http://host:port`. No TLS: the tray only talks to the local forge.
fn http_get(base: &str, path: &str) -> anyhow::Result<(u16, String)> {
    use std::io::{Read, Write};
    let rest = base
        .strip_prefix("http://")
        .ok_or_else(|| anyhow::anyhow!("FORGE_URL must be http://host:port (forge is local)"))?;
    let host = rest.split('/').next().unwrap_or(rest);
    let mut stream = std::net::TcpStream::connect(host)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(90)))?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {host}\r\nAccept: application/json\r\nConnection: close\r\n\r\n")?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("no HTTP response"))?;
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        dechunk(body)
    } else {
        body.to_string()
    };
    Ok((status, body))
}

fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some((size, after)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 || after.len() < n {
            break;
        }
        out.push_str(&after[..n]);
        rest = after[n..].trim_start_matches("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn feed() -> Value {
        json!({"stations": [
            {"label": "103_FIS_CLASSIFY", "family": "10_FIS", "path": "10_FIS\\103_FIS_CLASSIFY", "front": "C:/API/10_FIS/103_FIS_CLASSIFY",
             "buttons": [{"name": "1 RUN HERE", "path": "C:/API/10_FIS/103_FIS_CLASSIFY/1 RUN HERE.bat"}]},
            {"label": "101_FIS_INVENTORY", "family": "10_FIS", "path": "10_FIS\\101_FIS_INVENTORY", "front": "C:/API/10_FIS/101_FIS_INVENTORY",
             "buttons": [{"name": "1 RUN HERE", "path": "C:/API/10_FIS/101_FIS_INVENTORY/1 RUN HERE.bat"},
                         {"name": "notes", "path": "C:/API/10_FIS/101_FIS_INVENTORY/notes.txt"}]},
            {"label": "00_QUICK_CALL", "family": "", "path": "000_QUICK_CALL", "front": "C:/API/000_QUICK_CALL", "buttons": []},
            {"label": "60_OPENAI_STATIONS", "family": "", "path": "060_OPENAI_STATIONS", "front": "C:/API/060_OPENAI_STATIONS", "buttons": []},
            {"label": "91_CHAT", "family": "09_DESK_TOOLS", "path": "09_DESK_TOOLS\\091_CHAT", "front": "C:/API/09_DESK_TOOLS/091_CHAT", "buttons": []}
        ]})
    }

    #[test]
    fn groups_by_family_in_order_front_last_and_openai_hidden() {
        let g = build(&feed());
        let names: Vec<_> = g.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["09 · DESK TOOLS", "10 · FIS", FRONT]);
        assert_eq!(
            g[1].stations
                .iter()
                .map(|s| s.label.as_str())
                .collect::<Vec<_>>(),
            ["101_FIS_INVENTORY", "103_FIS_CLASSIFY"]
        );
        assert!(!render(&g).contains("OPENAI"));
        assert!(render(&g).contains("000_QUICK_CALL"));
    }

    #[test]
    fn only_bat_buttons_are_runnable() {
        let g = build(&feed());
        let inv = &g[1].stations[0];
        assert_eq!(inv.buttons.len(), 1);
        assert!(inv.buttons[0].path.ends_with(".bat"));
    }

    #[test]
    fn fetch_reads_a_local_forge_and_reports_errors() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for (i, stream) in listener.incoming().take(2).enumerate() {
                let mut s = stream.unwrap();
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let (status, body) = if i == 0 {
                    (
                        "200 OK",
                        r#"{"stations":[{"label":"101_FIS_INVENTORY","family":"10_FIS","path":"10_FIS/101_FIS_INVENTORY","buttons":[]}]}"#,
                    )
                } else {
                    (
                        "503 Service Unavailable",
                        r#"{"error":"pipeline not found"}"#,
                    )
                };
                let _ = write!(s, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
            }
        });
        let g = build(&fetch(&base).unwrap());
        assert_eq!(g[0].name, "10 · FIS");
        assert!(fetch(&base)
            .unwrap_err()
            .to_string()
            .contains("pipeline not found"));
        assert!(fetch("https://example.com").is_err());
    }

    #[test]
    fn dechunks() {
        assert_eq!(dechunk("4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n"), "Wikipedia");
    }

    #[test]
    fn bare_list_and_titles() {
        assert_eq!(build(&json!([])).len(), 0);
        assert_eq!(group_title("02_C_P_E_L"), "02 · C P E L");
        assert_eq!(group_title(""), FRONT);
        assert_eq!(group_title("_system"), "System");
    }
}
