//! The engine's bridge to mesh-workbench.
//!
//! mesh-workbench (crate `gpt-mesh`) edits a mesh through JSON commands on a
//! session that owns a workspace directory. This crate links it (ADR 0042)
//! and is the only engine crate that names it or `serde_json`: a caller
//! passes and receives [`genos_json::Value`].
//!
//! The session writes `<workspace>/project.json` after every command that
//! changes the mesh, and records each command under `<workspace>/sessions`.
//! `genos-workbench` reads those files for display.

use std::path::{Path, PathBuf};

use genos_json::Value;

/// The most commands one [`Bridge::execute`] call takes as a list.
/// mesh-workbench's own server takes the same.
pub const BATCH_MAX: usize = 1024;

/// An open mesh-workbench session on one workspace.
pub struct Bridge {
    session: gpt_mesh::session::Session,
    dir: PathBuf,
}

impl Bridge {
    /// Open the workspace at `dir`, creating the directory when it is missing.
    /// An existing `project.json` is loaded.
    ///
    /// # Errors
    /// The directory cannot be made, or its project does not load.
    pub fn open(dir: &Path) -> Result<Self, String> {
        let session = gpt_mesh::session::Session::open_workspace(dir.to_path_buf())
            .map_err(|err| format!("{}: {err}", dir.display()))?;
        Ok(Self { session, dir: dir.to_path_buf() })
    }

    /// The workspace directory this session owns.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Run one mesh-workbench command (`{"command": "...", ...}`) and return
    /// its reply (`{"ok": true, "result": ...}` or `{"ok": false, "error":
    /// ...}`). A list of commands runs in order and stops at the first that
    /// fails: the reply is `{"ok", "failed_command", "replies"}`, the shape
    /// mesh-workbench's server gives a batch.
    ///
    /// # Errors
    /// The request is not a command object or a list of 1 to [`BATCH_MAX`]
    /// of them. A command that mesh-workbench refuses is not an error here:
    /// its reply says `"ok": false`.
    pub fn execute(&mut self, request: &Value) -> Result<Value, String> {
        match request {
            Value::Object(_) => self.one(request),
            Value::Array(items) if (1..=BATCH_MAX).contains(&items.len()) => {
                let mut replies = Vec::with_capacity(items.len());
                let mut failed = None;
                for (index, item) in items.iter().enumerate() {
                    let reply = self.one(item)?;
                    let ok = reply.get("ok").and_then(Value::as_bool).unwrap_or(false);
                    replies.push(reply);
                    if !ok {
                        failed = Some(index);
                        break;
                    }
                }
                Ok(genos_json::object([
                    ("ok", genos_json::bool(failed.is_none())),
                    (
                        "failed_command",
                        failed.map_or(Value::Null, |index| {
                            genos_json::int(i64::try_from(index).unwrap_or(i64::MAX))
                        }),
                    ),
                    ("replies", genos_json::array(replies)),
                ]))
            }
            Value::Array(_) | Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                Err(format!("a mesh command is one command object or a list of 1 to {BATCH_MAX}"))
            }
        }
    }

    fn one(&mut self, request: &Value) -> Result<Value, String> {
        if !matches!(request, Value::Object(_)) {
            return Err("a mesh command is an object with a command field".into());
        }
        let theirs: serde_json::Value =
            serde_json::from_str(&genos_json::encode(request)).map_err(|err| format!("command: {err}"))?;
        let reply = self.session.execute(theirs);
        genos_json::parse(&reply.to_string()).map_err(|err| format!("reply: {err}"))
    }

    /// The document revision: it rises with every command that changes the mesh.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.session.state()["revision"].as_u64().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::Bridge;
    use genos_json::Value;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("genos-meshbench-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_box_command_changes_the_mesh_and_writes_the_project_file() {
        let dir = scratch("box");
        let mut bridge = Bridge::open(&dir).expect("open");
        assert_eq!(bridge.revision(), 0);
        let make = genos_json::parse(
            r#"{"command":"object_create_box","name":"crate","min":[0,0,0],"max":[1,1,1]}"#,
        )
        .expect("json");
        let reply = bridge.execute(&make).expect("execute");
        assert_eq!(reply.get("ok").and_then(Value::as_bool), Some(true), "{reply:?}");
        assert_eq!(bridge.revision(), 1);
        let project = std::fs::read_to_string(dir.join("project.json")).expect("project.json");
        let project = genos_json::parse(&project).expect("project json");
        assert_eq!(project.get("revision").and_then(Value::as_u64), Some(1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refused_command_replies_not_ok_and_a_batch_stops_there() {
        let dir = scratch("refused");
        let mut bridge = Bridge::open(&dir).expect("open");
        let batch = genos_json::parse(
            r#"[{"command":"object_create_box","name":"a","min":[0,0,0],"max":[1,1,1]},
                {"command":"no_such_command"},
                {"command":"object_create_box","name":"b","min":[2,0,0],"max":[3,1,1]}]"#,
        )
        .expect("json");
        let reply = bridge.execute(&batch).expect("execute");
        assert_eq!(reply.get("ok").and_then(Value::as_bool), Some(false));
        assert_eq!(reply.get("failed_command").and_then(Value::as_u64), Some(1));
        assert_eq!(reply.get("replies").and_then(Value::as_array).map(<[Value]>::len), Some(2));
        assert_eq!(bridge.revision(), 1, "the third command did not run");
        assert!(bridge.execute(&genos_json::parse("3").expect("json")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
