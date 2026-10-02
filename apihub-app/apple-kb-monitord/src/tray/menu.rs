//! `com.canonical.dbusmenu` at `/MenuBar`: `GetLayout`, `GetGroupProperties`
//! and `GetProperty` all implemented (GTK hosts such as waybar or the GNOME
//! AppIndicator extension use the last two; an empty answer there gives an
//! empty menu — defect D4 of docs/REVUE-UI-TRAY.md).

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use zbus::interface;
use zbus::zvariant::{OwnedValue, StructureBuilder, Value};

use super::view::{id, Entry, Prop, View};
use super::{lock, Action, Event, SharedRef};

/// One node of the wire layout `(ia{sv}av)`.
pub type Node = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

fn owned(v: Value<'static>) -> Option<OwnedValue> {
    OwnedValue::try_from(v).ok()
}

pub fn prop_value(p: &Prop) -> Option<OwnedValue> {
    match p {
        Prop::Str(s) => owned(Value::from(s.clone())),
        Prop::Bool(b) => owned(Value::from(*b)),
        Prop::Int(i) => owned(Value::from(*i)),
    }
}

fn wanted(names: &[String], k: &str) -> bool {
    names.is_empty() || names.iter().any(|n| n == k)
}

pub fn props_of(e: &Entry, names: &[String]) -> HashMap<String, OwnedValue> {
    e.props
        .iter()
        .filter(|(k, _)| wanted(names, k))
        .filter_map(|(k, v)| Some((k.to_string(), prop_value(v)?)))
        .collect()
}

fn root_props(names: &[String]) -> HashMap<String, OwnedValue> {
    let mut m = HashMap::new();
    if wanted(names, "children-display") {
        if let Some(v) = owned(Value::from("submenu")) {
            m.insert("children-display".into(), v);
        }
    }
    m
}

fn node_value(n: Node) -> Option<OwnedValue> {
    let s = StructureBuilder::new()
        .add_field(n.0)
        .add_field(n.1)
        .add_field(n.2)
        .build();
    owned(Value::Structure(s))
}

/// Layout from `parent` with `depth` levels of children (-1 = all).
/// The menu is flat: only the root has children.
pub fn layout(view: &View, parent: i32, depth: i32, names: &[String]) -> Option<Node> {
    if parent == id::ROOT {
        let children = if depth == 0 {
            Vec::new()
        } else {
            view.menu
                .iter()
                .filter_map(|e| node_value((e.id, props_of(e, names), Vec::new())))
                .collect()
        };
        return Some((id::ROOT, root_props(names), children));
    }
    view.entry(parent)
        .map(|e| (e.id, props_of(e, names), Vec::new()))
}

/// `GetGroupProperties`: empty `ids` = every item.
pub fn group_properties(
    view: &View,
    ids: &[i32],
    names: &[String],
) -> Vec<(i32, HashMap<String, OwnedValue>)> {
    let mut out = Vec::new();
    if ids.is_empty() || ids.contains(&id::ROOT) {
        out.push((id::ROOT, root_props(names)));
    }
    out.extend(
        view.menu
            .iter()
            .filter(|e| ids.is_empty() || ids.contains(&e.id))
            .map(|e| (e.id, props_of(e, names))),
    );
    out
}

/// Differences between two menus, as dbusmenu wants them:
/// `Some(true)` = structure (visibility) changed → `LayoutUpdated`;
/// otherwise the changed / removed properties for `ItemsPropertiesUpdated`.
pub enum MenuDiff {
    Same,
    Layout,
    Props {
        updated: Vec<(i32, HashMap<String, OwnedValue>)>,
        removed: Vec<(i32, Vec<String>)>,
    },
}

pub fn diff(old: &View, new: &View) -> MenuDiff {
    if old.menu == new.menu {
        return MenuDiff::Same;
    }
    let vis =
        |v: &View| -> Vec<(i32, bool)> { v.menu.iter().map(|e| (e.id, e.visible())).collect() };
    if vis(old) != vis(new) {
        return MenuDiff::Layout;
    }
    let mut updated = Vec::new();
    let mut removed = Vec::new();
    for (o, n) in old.menu.iter().zip(new.menu.iter()) {
        if o == n {
            continue;
        }
        let changed: HashMap<String, OwnedValue> = n
            .props
            .iter()
            .filter(|(k, v)| o.get(k) != Some(v))
            .filter_map(|(k, v)| Some((k.to_string(), prop_value(v)?)))
            .collect();
        let gone: Vec<String> = o
            .props
            .iter()
            .filter(|(k, _)| n.get(k).is_none())
            .map(|(k, _)| k.to_string())
            .collect();
        if !changed.is_empty() {
            updated.push((n.id, changed));
        }
        if !gone.is_empty() {
            removed.push((n.id, gone));
        }
    }
    MenuDiff::Props { updated, removed }
}

pub struct Menu {
    pub shared: SharedRef,
    pub tx: Sender<Event>,
}

impl Menu {
    fn click(&self, item: i32) {
        let action = match item {
            id::OPEN => Action::Open,
            id::REFRESH => Action::Refresh,
            id::COPY => Action::Copy,
            id::BLUETOOTH => Action::Bluetooth,
            id::RENAME => Action::Rename,
            id::REPAIR => Action::Repair,
            id::FN_MEDIA => Action::FnMode(super::view::FN_MEDIA_FIRST),
            id::FN_FKEYS => Action::FnMode(super::view::FN_FKEYS_FIRST),
            id::QUIT => Action::Hide,
            _ => return,
        };
        let _ = self.tx.send(Event::Action(action));
    }
}

#[interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        let p = lock(&self.shared).icon_theme_path.clone();
        if p.is_empty() {
            Vec::new()
        } else {
            vec![p]
        }
    }

    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, Node)> {
        let s = lock(&self.shared);
        layout(&s.view, parent_id, recursion_depth, &property_names)
            .map(|n| (s.menu_rev, n))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("unknown item {parent_id}")))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        group_properties(&lock(&self.shared).view, &ids, &property_names)
    }

    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedValue> {
        let s = lock(&self.shared);
        let names = [name.to_string()];
        let props = if id == id::ROOT {
            root_props(&names)
        } else {
            s.view
                .entry(id)
                .map(|e| props_of(e, &names))
                .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("unknown item {id}")))?
        };
        props
            .into_values()
            .next()
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("item {id} has no {name}")))
    }

    fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
        if event_id == "clicked" {
            self.click(id);
        }
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        let s_ids: Vec<i32> = lock(&self.shared).view.menu.iter().map(|e| e.id).collect();
        let mut errors = Vec::new();
        for (id, event_id, _, _) in &events {
            if *id != id::ROOT && !s_ids.contains(id) {
                errors.push(*id);
            } else if event_id == "clicked" {
                self.click(*id);
            }
        }
        errors
    }

    /// The menu is always up to date (updates are pushed by signals).
    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }

    #[zbus(signal)]
    pub async fn layout_updated(
        ctxt: &zbus::object_server::SignalContext<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn items_properties_updated(
        ctxt: &zbus::object_server::SignalContext<'_>,
        updated_props: Vec<(i32, HashMap<String, OwnedValue>)>,
        removed_props: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn item_activation_requested(
        ctxt: &zbus::object_server::SignalContext<'_>,
        id: i32,
        timestamp: u32,
    ) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::super::view::Lang;
    use super::*;
    use akm_core::{KbReport, Snapshot};

    fn view(pct: f64, connected: bool) -> View {
        let mut k = KbReport::default();
        k.battery.percentage_fine = Some(pct);
        k.device.model = Some("Magic Keyboard".into());
        let s = Snapshot {
            connected,
            keyboard: Some(k),
            ..Default::default()
        };
        View::build(&s, false, None, Lang::En)
    }

    #[test]
    fn group_properties_is_never_empty() {
        let v = view(80.0, true);
        let all = group_properties(&v, &[], &[]);
        assert_eq!(all.len(), 1 + id::ALL.len());
        assert!(all
            .iter()
            .filter(|(i, _)| *i != id::ROOT)
            .all(|(_, p)| !p.is_empty()));
        let one = group_properties(&v, &[id::OPEN], &["label".into()]);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].0, id::OPEN);
        assert_eq!(one[0].1.len(), 1);
        let label: String = one[0].1["label"].try_clone().unwrap().try_into().unwrap();
        assert_eq!(label, "_Open window…");
    }

    #[test]
    fn layout_root_and_items() {
        let v = view(80.0, true);
        let (rid, rp, children) = layout(&v, id::ROOT, -1, &[]).unwrap();
        assert_eq!(rid, 0);
        assert!(rp.contains_key("children-display"));
        assert_eq!(children.len(), id::ALL.len());
        let (_, _, none) = layout(&v, id::ROOT, 0, &[]).unwrap();
        assert!(none.is_empty());
        let (i, p, c) = layout(&v, id::QUIT, -1, &[]).unwrap();
        assert_eq!((i, c.len()), (id::QUIT, 0));
        assert!(p.contains_key("label"));
        assert!(layout(&v, 999, -1, &[]).is_none());
    }

    #[test]
    fn diff_kinds() {
        let a = view(80.0, true);
        assert!(matches!(diff(&a, &a), MenuDiff::Same));
        // 80 → 79: only the battery label changes.
        match diff(&a, &view(79.0, true)) {
            MenuDiff::Props { updated, removed } => {
                assert_eq!(updated.len(), 1);
                assert_eq!(updated[0].0, id::BATTERY);
                assert!(removed.is_empty());
            }
            _ => panic!("expected a property update"),
        }
        // Disconnection hides the battery line: structure change.
        assert!(matches!(diff(&a, &view(80.0, false)), MenuDiff::Layout));
        // 25 → 15: disposition appears (update), back: disposition removed.
        let w = view(15.0, true);
        match diff(&w, &view(25.0, true)) {
            MenuDiff::Props { removed, .. } => {
                assert_eq!(
                    removed,
                    vec![(id::BATTERY, vec!["disposition".to_string()])]
                );
            }
            _ => panic!("expected removed props"),
        }
    }
}
