use crate::mux::*;

#[test]
fn test_mux_new() {
    let mux = Mux::new(24, 80);
    assert_eq!(mux.size(), (24, 80));
    assert!(mux.focused().is_none());
}

#[test]
fn test_mux_add_pane() {
    let mut mux = Mux::new(24, 80);
    let pane = Pane::director("test", 24, 80).unwrap();
    mux.add_pane(pane);

    assert!(mux.get("test").is_some());
    assert_eq!(mux.focused_id(), Some("test"));
}

#[test]
fn test_mux_focus_navigation() {
    let mut mux = Mux::new(24, 80);
    mux.add_pane(Pane::director("pane1", 24, 40).unwrap());
    mux.add_pane(Pane::director("pane2", 24, 40).unwrap());

    assert_eq!(mux.focused_id(), Some("pane1"));

    mux.focus_next();
    assert_eq!(mux.focused_id(), Some("pane2"));

    mux.focus_next();
    assert_eq!(mux.focused_id(), Some("pane1")); // Wraps around

    mux.focus_prev();
    assert_eq!(mux.focused_id(), Some("pane2"));
}

#[test]
fn test_pane_count() {
    let mut mux = Mux::new(24, 80);
    assert_eq!(mux.pane_count(), 0);

    mux.add_pane(Pane::director("pane1", 24, 40).unwrap());
    assert_eq!(mux.pane_count(), 1);

    mux.add_pane(Pane::director("pane2", 24, 40).unwrap());
    assert_eq!(mux.pane_count(), 2);

    mux.remove_pane("pane1");
    assert_eq!(mux.pane_count(), 1);
}

#[test]
fn test_remove_pane_focus_transfer() {
    let mut mux = Mux::new(24, 80);
    mux.add_pane(Pane::director("pane1", 24, 40).unwrap());
    mux.add_pane(Pane::director("pane2", 24, 40).unwrap());

    // Focus is on pane1 (first added)
    assert_eq!(mux.focused_id(), Some("pane1"));

    // Remove focused pane, focus should transfer to next
    mux.remove_pane("pane1");
    assert_eq!(mux.focused_id(), Some("pane2"));
    assert_eq!(mux.pane_count(), 1);
}
