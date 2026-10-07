//! The accessibility inspector: the screen's accessibility tree, as a screen
//! reader sees it, and checks for common accessibility problems.
//!
//! AAE's helper reads the tree inside the device (it's an accessibility
//! service, like TalkBack) and sends it as JSON.

use serde::{Deserialize, Serialize};

use crate::adb::Adb;
use crate::error::{Error, Result};

const HELPER_RECEIVER: &str = "io.github.aaron_gh.aae.helper/.CommandReceiver";
const DUMP_TREE: &str = "io.github.aaron_gh.aae.helper.DUMP_TREE";
/// The smallest touch target Android's accessibility guidelines recommend, in dp.
const MIN_TOUCH_DP: f64 = 48.0;

/// The whole screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tree {
    /// Screen pixels per dp.
    pub density: f64,
    pub api: u32,
    /// The screen had more elements than the helper sends.
    #[serde(default)]
    pub truncated: bool,
    pub windows: Vec<Window>,
}

/// One window, such as the app, a dialog, the keyboard or the navigation bar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Window {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub focused: bool,
    pub root: Node,
}

/// One element of the screen, with what a screen reader reads from it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub pane: Option<String>,
    #[serde(default)]
    pub tooltip: Option<String>,
    #[serde(default)]
    pub heading: bool,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub labelled_by: Option<String>,
    /// Left, top, right, bottom, in screen pixels.
    #[serde(default)]
    pub bounds: [i32; 4],
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub collection: Option<String>,
    #[serde(default)]
    pub item: Option<String>,
    #[serde(default)]
    pub range: Option<String>,
    #[serde(default)]
    pub children: Vec<Node>,
}

impl Node {
    pub fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }

    /// The class name without its package, such as "Button".
    pub fn short_class(&self) -> &str {
        let class = self.class.as_deref().unwrap_or("View");
        class.rsplit('.').next().unwrap_or(class)
    }

    /// The kind of element in words, roughly as TalkBack says it.
    pub fn kind(&self) -> Option<String> {
        if let Some(role) = &self.role {
            return Some(role.clone());
        }
        let class = self.short_class();
        let kind = match class {
            "Button" | "ImageButton" | "FloatingActionButton" | "MaterialButton" => "button",
            "CheckBox" | "MaterialCheckBox" => "checkbox",
            "Switch" | "SwitchCompat" | "SwitchMaterial" => "switch",
            "RadioButton" => "radio button",
            "ToggleButton" => "toggle button",
            "EditText" | "AutoCompleteTextView" | "TextInputEditText" => "edit box",
            "ImageView" => "image",
            "SeekBar" | "Slider" => "slider",
            "ProgressBar" => "progress bar",
            "Spinner" => "drop-down list",
            "TabWidget" | "TabLayout" => "tabs",
            "RecyclerView"
            | "ListView"
            | "GridView"
            | "ScrollView"
            | "NestedScrollView"
            | "HorizontalScrollView" => "list",
            "WebView" => "web view",
            _ if self.has("checkable") => "checkbox",
            _ if self.has("editable") => "edit box",
            _ => return None,
        };
        Some(kind.to_string())
    }

    /// The element's own label: its content description, else its text, else
    /// the element labelling it.
    pub fn own_label(&self) -> Option<&str> {
        [&self.description, &self.text, &self.labelled_by]
            .into_iter()
            .flatten()
            .map(|s| s.trim())
            .find(|s| !s.is_empty())
    }

    /// The label a screen reader would speak: the element's own label, or, for
    /// an element it can focus or activate, the text of the elements inside it,
    /// joined as TalkBack does. Plain layout containers have none.
    pub fn spoken_label(&self) -> String {
        if let Some(label) = self.own_label() {
            return label.to_string();
        }
        // Lists and other scrolling containers are announced as what they
        // are, not by everything in them.
        let container = self.has("scrollable") || self.collection.is_some();
        if container || !(self.is_control() || self.has("focusable")) {
            return String::new();
        }
        let mut parts = Vec::new();
        collect_text(&self.children, &mut parts);
        parts.join(", ")
    }

    /// A one-line description, such as "Send, button, disabled".
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        let label = self.spoken_label();
        if !label.is_empty() {
            parts.push(label);
        }
        if let Some(state) = &self.state {
            parts.push(state.clone());
        }
        if let Some(kind) = self.kind() {
            parts.push(kind);
        }
        if let Some(collection) = &self.collection {
            parts.push(collection.clone());
        }
        if self.heading {
            parts.push("heading".into());
        }
        if self.has("checkable") {
            parts.push(
                if self.has("checked") {
                    "checked"
                } else {
                    "not checked"
                }
                .into(),
            );
        }
        if self.has("selected") {
            parts.push("selected".into());
        }
        if self.has("disabled") {
            parts.push("disabled".into());
        }
        if let Some(hint) = &self.hint {
            if self.text.as_deref() != Some(hint) {
                parts.push(format!("hint {hint}"));
            }
        }
        if let Some(error) = &self.error {
            parts.push(format!("error {error}"));
        }
        if parts.is_empty() {
            parts.push(self.short_class().to_string());
        }
        if self.has("accessibility focused") {
            parts.push("screen reader focus".into());
        }
        // One line, even when the text has line breaks.
        parts
            .join(", ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Every property, one per line, for the details view.
    pub fn details(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let mut add = |name: &str, value: &Option<String>| {
            if let Some(value) = value {
                lines.push(format!("{name}: {value}"));
            }
        };
        add("Text", &self.text);
        add("Content description", &self.description);
        add("Labelled by", &self.labelled_by);
        add("Hint", &self.hint);
        add("State", &self.state);
        add("Role", &self.role);
        add("Pane title", &self.pane);
        add("Tooltip", &self.tooltip);
        add("Error", &self.error);
        add("Class", &self.class);
        add("Resource ID", &self.id);
        add("Collection", &self.collection);
        add("Collection item", &self.item);
        add("Range", &self.range);
        if self.heading {
            lines.push("Heading: yes".into());
        }
        let [l, t, r, b] = self.bounds;
        lines.push(format!(
            "Position: {l}, {t}; size {} by {} pixels",
            r - l,
            b - t
        ));
        if !self.flags.is_empty() {
            lines.push(format!("Flags: {}", self.flags.join(", ")));
        }
        if !self.actions.is_empty() {
            lines.push(format!("Actions: {}", self.actions.join(", ")));
        }
        lines
    }

    pub fn is_control(&self) -> bool {
        self.has("clickable")
            || self.has("long clickable")
            || self.has("checkable")
            || self.has("editable")
    }
}

fn collect_text(nodes: &[Node], parts: &mut Vec<String>) {
    for node in nodes {
        if let Some(label) = node.own_label() {
            parts.push(label.to_string());
        } else {
            collect_text(&node.children, parts);
        }
    }
}

impl Window {
    /// Such as "Application window: Settings".
    pub fn describe(&self) -> String {
        let kind = match self.kind.as_str() {
            "" => "Window".to_string(),
            kind => format!("{}{} window", kind[..1].to_uppercase(), &kind[1..]),
        };
        let mut text = match &self.title {
            Some(title) if !title.is_empty() => format!("{kind}: {title}"),
            _ => kind,
        };
        if self.active {
            text.push_str(", active");
        }
        text
    }
}

/// Something on the screen that can be touched, for moving a touch point
/// from item to item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Target {
    /// What a screen reader would say, such as "Send, button".
    pub label: String,
    /// Left, top, right, bottom, in pixels of the screen as the user sees it,
    /// cut to what's on screen.
    pub bounds: [i32; 4],
}

impl Target {
    pub fn centre(&self) -> (i32, i32) {
        let [l, t, r, b] = self.bounds;
        ((l + r) / 2, (t + b) / 2)
    }

    fn contains(&self, x: i32, y: i32) -> bool {
        let [l, t, r, b] = self.bounds;
        (l..r).contains(&x) && (t..b).contains(&y)
    }

    fn area(&self) -> i64 {
        let [l, t, r, b] = self.bounds;
        (r - l) as i64 * (b - t) as i64
    }
}

/// The things on the screen worth touching, in reading order: the active
/// app first, then the rest, such as the keyboard and the system bars.
///
/// As a screen reader does, a control is one stop, with the text inside it
/// part of its label; text outside any control is a stop of its own.
/// Unlabelled controls are included, since finding them is the point of
/// testing, but a control that covers most of the screen, usually a layout
/// that happens to be clickable, is not.
pub fn targets(tree: &Tree) -> Vec<Target> {
    fn visible(node: &Node) -> bool {
        let [l, t, r, b] = node.bounds;
        r > l && b > t
    }
    fn walk(node: &Node, inside_control: bool, screen_area: i64, out: &mut Vec<Target>) {
        let control = node.is_control() || node.has("focusable");
        let target = Target {
            label: node.summary(),
            bounds: node.bounds,
        };
        let worth = visible(node)
            && if control {
                target.area() < screen_area * 8 / 10
            } else {
                !inside_control && node.own_label().is_some()
            };
        if worth {
            out.push(target);
        }
        for child in &node.children {
            walk(
                child,
                inside_control || (control && worth),
                screen_area,
                out,
            );
        }
    }
    let mut windows: Vec<&Window> = tree.windows.iter().collect();
    // Stable sort: the active window, then other app windows, keep their order.
    windows.sort_by_key(|w| (!w.active, w.kind != "application"));
    let mut out = Vec::new();
    for window in windows {
        let [l, t, r, b] = window.root.bounds;
        let area = ((r - l) as i64 * (b - t) as i64).max(1);
        walk(&window.root, false, area, &mut out);
    }
    out
}

/// What's under a point: the smallest target containing it.
pub fn target_at(targets: &[Target], x: i32, y: i32) -> Option<&Target> {
    targets
        .iter()
        .filter(|t| t.contains(x, y))
        .min_by_key(|t| t.area())
}

/// Reads the screen's accessibility tree through AAE's helper. Just after
/// the helper is turned on, Android takes a moment to connect it, so this
/// waits up to three seconds for it.
pub async fn read_tree(adb: &Adb) -> Result<Tree> {
    let mut out = String::new();
    for attempt in 0..12 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        out = adb
            .shell(&format!("am broadcast -n {HELPER_RECEIVER} -a {DUMP_TREE}"))
            .await?;
        if out.contains("result=1") {
            break;
        }
    }
    if !out.contains("result=1") {
        return Err(Error::Adb(
            "AAE's helper service isn't running, so it can't read the screen. Start the device again to turn it on."
                .into(),
        ));
    }
    let json = out
        .split_once("data=\"")
        .and_then(|(_, rest)| rest.rsplit_once('"'))
        .map(|(json, _)| json)
        .ok_or_else(|| Error::Adb("AAE's helper sent no screen contents.".into()))?;
    serde_json::from_str(json).map_err(|e| {
        Error::Adb(format!(
            "AAE's helper sent screen contents AAE can't read: {e}"
        ))
    })
}

/// The tree as indented text, one element per line.
pub fn to_text(tree: &Tree) -> String {
    fn walk(node: &Node, depth: usize, out: &mut String) {
        out.push_str(&"  ".repeat(depth));
        out.push_str(&node.summary());
        out.push('\n');
        for child in &node.children {
            walk(child, depth + 1, out);
        }
    }
    let mut out = String::new();
    for window in &tree.windows {
        out.push_str(&window.describe());
        out.push('\n');
        walk(&window.root, 1, &mut out);
    }
    if tree.truncated {
        out.push_str("The screen has more elements than shown.\n");
    }
    out
}

/// How serious an accessibility issue is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Severity {
    Error,
    Warning,
}

/// An accessibility problem found on the screen.
#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub severity: Severity,
    /// What's wrong, in words.
    pub message: String,
    /// The element it's about, as [`Node::summary`] describes it.
    pub element: String,
    /// The element's resource ID, if it has one, to find it in the code.
    pub id: Option<String>,
}

/// Checks the screen's application windows for common accessibility problems.
/// System windows, such as the navigation bar, aren't the app's to fix.
pub fn check(tree: &Tree) -> Vec<Issue> {
    let mut issues = Vec::new();
    for window in tree
        .windows
        .iter()
        .filter(|w| w.kind == "application" || w.kind.is_empty())
    {
        check_node(&window.root, tree.density, window.root.bounds, &mut issues);
    }
    issues.sort_by_key(|i| i.severity);
    issues
}

/// `screen` is the window's bounds: elements cut off at its edge only report
/// their visible part, so their size can't be judged.
fn check_node(node: &Node, density: f64, screen: [i32; 4], issues: &mut Vec<Issue>) {
    let visible = !node.has("not visible") && !node.has("not important");
    let mut issue = |severity, message: String| {
        issues.push(Issue {
            severity,
            message,
            element: node.summary(),
            id: node.id.clone(),
        });
    };
    if visible && node.is_control() && node.spoken_label().is_empty() && node.hint.is_none() {
        issue(
            Severity::Error,
            "This control has no label, so a screen reader can't say what it does.".into(),
        );
    }
    if visible
        && node.short_class().contains("Image")
        && !node.is_control()
        && node.own_label().is_none()
        && node.has("focusable")
    {
        issue(
            Severity::Warning,
            "This image has no description. If it carries meaning, describe it; if it's decoration, hide it from screen readers.".into(),
        );
    }
    let [l, t, r, b] = node.bounds;
    let cut_off = l <= screen[0] || t <= screen[1] || r >= screen[2] || b >= screen[3];
    if visible && (node.has("clickable") || node.has("long clickable")) && density > 0.0 && !cut_off
    {
        let (w, h) = (
            ((r - l) as f64 / density).round(),
            ((b - t) as f64 / density).round(),
        );
        if (w > 0.0 && w < MIN_TOUCH_DP) || (h > 0.0 && h < MIN_TOUCH_DP) {
            issue(
                Severity::Warning,
                format!(
                    "This touch target is {w} by {h} dp. At least {MIN_TOUCH_DP} by {MIN_TOUCH_DP} is recommended."
                ),
            );
        }
    }
    // Several controls side by side with the same label can't be told apart.
    let mut seen: Vec<(String, usize)> = Vec::new();
    for child in node.children.iter().filter(|c| c.is_control()) {
        let label = child.spoken_label();
        if label.is_empty() {
            continue;
        }
        match seen.iter_mut().find(|(l, _)| *l == label) {
            Some((_, count)) => *count += 1,
            None => seen.push((label, 1)),
        }
    }
    for (label, count) in seen.into_iter().filter(|(_, count)| *count > 1) {
        issues.push(Issue {
            severity: Severity::Warning,
            message: format!(
                "{count} controls here are all labelled \"{label}\", so they can't be told apart."
            ),
            element: node.summary(),
            id: node.id.clone(),
        });
    }
    for child in &node.children {
        check_node(child, density, screen, issues);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(json: &str) -> Tree {
        serde_json::from_str(json).unwrap()
    }

    const SCREEN: &str = r#"{"density":2.0,"api":36,"windows":[{"type":"application","title":"Mail","active":true,"root":
        {"class":"android.widget.FrameLayout","bounds":[0,0,1080,2400],"flags":[],"actions":[],"children":[
          {"class":"android.widget.ImageButton","bounds":[10,10,70,70],"flags":["clickable","focusable"],"actions":["click"]},
          {"class":"android.widget.Button","text":"Send","bounds":[100,0,400,200],"flags":["clickable","focusable","disabled"],"actions":[]},
          {"class":"android.widget.Button","text":"More","bounds":[0,300,300,500],"flags":["clickable"],"actions":[]},
          {"class":"android.widget.Button","text":"More","bounds":[0,600,300,800],"flags":["clickable"],"actions":[]},
          {"class":"android.widget.LinearLayout","bounds":[0,900,800,1100],"flags":["clickable"],"actions":[],"children":[
            {"class":"android.widget.TextView","text":"Inbox","bounds":[0,900,400,1100],"flags":[],"actions":[]},
            {"class":"android.widget.TextView","text":"3 unread","bounds":[400,900,800,1100],"flags":[],"actions":[]}]}
        ]}}]}"#;

    #[test]
    fn summarises_like_a_screen_reader() {
        let tree = tree(SCREEN);
        let root = &tree.windows[0].root;
        assert_eq!(root.children[1].summary(), "Send, button, disabled");
        assert_eq!(root.children[4].summary(), "Inbox, 3 unread");
        assert!(to_text(&tree).starts_with("Application window: Mail, active\n"));
    }

    #[test]
    fn finds_issues() {
        let issues = check(&tree(SCREEN));
        assert!(issues[0].severity == Severity::Error && issues[0].message.contains("no label"));
        assert!(issues.iter().any(|i| i.message.contains("30 by 30 dp")));
        assert!(issues.iter().any(|i| {
            i.message
                .contains("2 controls here are all labelled \"More\"")
        }));
        assert!(
            !issues.iter().any(|i| i.element.starts_with("Inbox")),
            "labelled by its children"
        );
    }
}
