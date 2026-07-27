use crate::ipc::protocol::{IpcQuery, IpcResponse};
use crate::layout::WindowState;
use crate::state::WMState;

pub(crate) fn handle_query(query: IpcQuery, state: &WMState) -> IpcResponse {
    let data = match query {
        IpcQuery::Windows => snapshot_windows(state),
        IpcQuery::Outputs => snapshot_outputs(state),
        IpcQuery::Focused => snapshot_focused(state),
        IpcQuery::All => {
            serde_json::json!({
                "windows": snapshot_windows(state),
                "outputs": snapshot_outputs(state),
                "focused": snapshot_focused(state),
            })
        }
    };
    IpcResponse::success(data)
}

fn state_tag(state: &WindowState) -> &'static str {
    match state {
        WindowState::Tiled => "tiled",
        WindowState::Floating { .. } => "floating",
        WindowState::PseudoTiled { .. } => "pseudo_tiled",
        WindowState::Fullscreen { .. } => "fullscreen",
    }
}

fn snapshot_windows(state: &WMState) -> serde_json::Value {
    let mut windows = Vec::new();
    for output_id in state.outputs.keys() {
        if let Some(tree) = state.output_trees.get(output_id) {
            for (win_id, rect, win_state) in tree.arranged_windows_readonly() {
                let wid = crate::state::window::WindowId(win_id);
                let window = state.windows.get(&wid);
                let title = window.and_then(|w| w.title.as_deref()).unwrap_or("");
                let app_id = window.and_then(|w| w.app_id.as_deref()).unwrap_or("");
                let pid = window.map(|w| w.pid).unwrap_or(0);
                windows.push(serde_json::json!({
                    "id": win_id,
                    "output_id": output_id.0,
                    "title": title,
                    "app_id": app_id,
                    "pid": pid,
                    "rect": {
                        "x": rect.x,
                        "y": rect.y,
                        "w": rect.width,
                        "h": rect.height,
                    },
                    "state": state_tag(&win_state),
                }));
            }
        }
    }
    serde_json::Value::Array(windows)
}

fn snapshot_outputs(state: &WMState) -> serde_json::Value {
    let mut outputs = Vec::new();
    for (output_id, output) in state.outputs.iter() {
        let rect = output
            .rect()
            .map(|r| serde_json::json!({"x": r.x, "y": r.y, "w": r.width, "h": r.height}));
        let tiling_rect = output
            .tiling_rect()
            .map(|r| serde_json::json!({ "x": r.x, "y": r.y, "w": r.width, "h": r.height }));
        let focused_window = state
            .output_trees
            .get(output_id)
            .and_then(|t| t.focused_window());
        outputs.push(serde_json::json!({
            "id": output_id.0,
            "rect": rect,
            "work_area": tiling_rect,
            "focused_window": focused_window,
        }));
    }
    serde_json::Value::Array(outputs)
}

fn snapshot_focused(state: &WMState) -> serde_json::Value {
    let focus_stack: Vec<u32> = state.focus_stack.iter().map(|id| id.0).collect();
    serde_json::json!({
        "window_id": state.focused_window.map(|id| id.0),
        "output_id": state.focused_output.map(|id| id.0),
        "focus_stack": focus_stack,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{
        output::Output,
        output::OutputId,
        window::{Window, WindowId},
    };

    #[test]
    fn snapshot_windows_returns_all_windows() {
        let mut state = WMState::new();
        let oid = OutputId(1);
        state.outputs.insert(oid, Output::new());
        state.outputs.get_mut(&oid).unwrap().dimensions = Some(crate::state::window::Dimensions {
            width: 1920,
            height: 1080,
        });
        let wid = WindowId(1);
        state.windows.insert(wid, Window::new(wid, oid));
        state.tree_for_output(oid).unwrap().insert_window(1, None);
        state.push_focus(wid);
        state.tree_for_output(oid).unwrap().arranged_windows();

        let windows = snapshot_windows(&state);
        let arr = windows.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], 1);
        assert_eq!(arr[0]["state"], "tiled");
        assert_eq!(arr[0]["rect"]["x"], 0);
        assert_eq!(arr[0]["rect"]["y"], 0);
        assert!(arr[0]["rect"]["w"].as_i64().unwrap() > 0);
        assert!(arr[0]["rect"]["h"].as_i64().unwrap() > 0);
    }

    #[test]
    fn snapshot_focused_returns_current_focus() {
        let mut state = WMState::new();
        let oid = OutputId(1);
        state.outputs.insert(oid, Output::new());
        let wid = WindowId(1);
        state.windows.insert(wid, Window::new(wid, oid));
        state.tree_for_output(oid).unwrap().insert_window(1, None);
        state.focus_window_id(wid);

        let focused = snapshot_focused(&state);
        assert_eq!(focused["window_id"], 1);
        assert_eq!(focused["output_id"], 1);
    }

    #[test]
    fn snapshot_outputs_with_geometry_reports_rect_and_work_area() {
        let mut state = WMState::new();
        let oid = OutputId(1);
        let mut output = Output::new();
        output.dimensions = Some(crate::state::window::Dimensions {
            width: 1920,
            height: 1080,
        });
        output.position = Some((20, 10));
        state.outputs.insert(oid, output);
        let wid = WindowId(1);
        state.windows.insert(wid, Window::new(wid, oid));
        state.tree_for_output(oid).unwrap().insert_window(1, None);
        state.focus_window_id(wid);

        let outputs = snapshot_outputs(&state);
        let arr = outputs.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], 1);
        assert_eq!(arr[0]["rect"]["x"], 20);
        assert_eq!(arr[0]["rect"]["y"], 10);
        assert_eq!(arr[0]["rect"]["w"], 1920);
        assert_eq!(arr[0]["rect"]["h"], 1080);
        assert_eq!(arr[0]["work_area"]["x"], 20);
        assert_eq!(arr[0]["work_area"]["y"], 10);
        assert_eq!(arr[0]["work_area"]["w"], 1920);
        assert_eq!(arr[0]["work_area"]["h"], 1080);
        assert_eq!(arr[0]["focused_window"].as_u64(), Some(1));
    }

    #[test]
    fn snapshot_outputs_without_geometry_has_null_rects() {
        let state = WMState::new();
        let oid = OutputId(1);
        // Output has no dimensions or position — rect() and tiling_rect() return None.
        let outputs = {
            let mut s = state;
            s.outputs.insert(oid, Output::new());
            snapshot_outputs(&s)
        };
        let arr = outputs.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], 1);
        assert!(arr[0]["rect"].is_null());
        assert!(arr[0]["work_area"].is_null());
        assert!(arr[0]["focused_window"].is_null());
    }

    #[test]
    fn snapshot_outputs_lists_multiple_outputs() {
        let mut state = WMState::new();
        let o1 = OutputId(1);
        let o2 = OutputId(2);
        state.outputs.insert(o1, Output::new());
        state.outputs.insert(o2, Output::new());

        let outputs = snapshot_outputs(&state);
        let arr = outputs.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        let ids: Vec<u64> = arr.iter().map(|v| v["id"].as_u64().unwrap()).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
    }
}
