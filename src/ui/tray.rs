use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub struct StatusTray {
    pub _tray_icon: TrayIcon,
    pub type_item: MenuItem,
    pub quit_item: MenuItem,
}

pub fn build_status_tray() -> Result<StatusTray, String> {
    let menu = Menu::new();
    let type_item = MenuItem::new("Type last transcript", false, None);
    let separator = PredefinedMenuItem::separator();
    let quit_item = MenuItem::new("Quit", true, None);

    menu.append(&type_item)
        .map_err(|e| format!("Failed to append type item: {e}"))?;
    menu.append(&separator)
        .map_err(|e| format!("Failed to append separator: {e}"))?;
    menu.append(&quit_item)
        .map_err(|e| format!("Failed to append quit item: {e}"))?;

    let tray_icon = TrayIconBuilder::new()
        .with_tooltip("Voxtral Speech-to-Text")
        .with_icon(build_status_icon()?)
        .with_icon_as_template(true)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(true)
        .build()
        .map_err(|e| format!("Failed to build tray icon: {e}"))?;

    Ok(StatusTray {
        _tray_icon: tray_icon,
        type_item,
        quit_item,
    })
}

fn build_status_icon() -> Result<Icon, String> {
    let icon = crate::ui::tray_art::build_status_icon_rgba();
    Icon::from_rgba(icon.rgba, icon.width, icon.height)
        .map_err(|e| format!("Failed to create tray icon RGBA data: {e}"))
}
