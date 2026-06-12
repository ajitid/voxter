use crate::{AppEvent, AppSender, ControlMsg};
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{Category, Icon, MenuItem, Status, ToolTip, Tray};

pub struct LinuxStatusTray {
    handle: Handle<VoxterLinuxTray>,
}

impl LinuxStatusTray {
    pub fn new(sender: AppSender, has_last_transcript: bool) -> Result<Self, String> {
        let tray = VoxterLinuxTray {
            sender,
            has_last_transcript,
            use_light_pixmap: is_gnome_session(),
        };
        let handle = tray
            .spawn()
            .map_err(|e| format!("Failed to register StatusNotifierItem: {e}"))?;
        Ok(Self { handle })
    }

    pub fn refresh_type_item(&self, has_last_transcript: bool) {
        let _ = self.handle.update(|tray| {
            tray.has_last_transcript = has_last_transcript;
        });
    }

    pub fn shutdown(&self) {
        self.handle.shutdown().wait();
    }
}

struct VoxterLinuxTray {
    sender: AppSender,
    has_last_transcript: bool,
    use_light_pixmap: bool,
}

fn is_gnome_session() -> bool {
    ["XDG_CURRENT_DESKTOP", "DESKTOP_SESSION"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .any(|value| value.to_ascii_lowercase().contains("gnome"))
}

fn build_light_status_icon_pixmap() -> Icon {
    let icon = crate::ui::tray_art::build_status_icon_rgba();
    let mut data = Vec::with_capacity(icon.rgba.len());

    for pixel in icon.rgba.chunks_exact(4) {
        let alpha = pixel[3];
        data.extend_from_slice(&[alpha, 255, 255, 255]);
    }

    Icon {
        width: icon.width as i32,
        height: icon.height as i32,
        data,
    }
}

impl Tray for VoxterLinuxTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "voxter".to_string()
    }

    fn title(&self) -> String {
        "Voxter".to_string()
    }

    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    fn status(&self) -> Status {
        Status::Active
    }

    fn icon_name(&self) -> String {
        if self.use_light_pixmap {
            String::new()
        } else {
            "voxter-symbolic".to_string()
        }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        if self.use_light_pixmap {
            vec![build_light_status_icon_pixmap()]
        } else {
            Vec::new()
        }
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Voxter".to_string(),
            description: "Mistral Voxtral Speech-to-Text".to_string(),
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Type last transcript".to_string(),
                enabled: self.has_last_transcript,
                activate: Box::new(|tray: &mut Self| {
                    tray.sender
                        .send(AppEvent::Control(ControlMsg::TypeLastTranscript));
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".to_string(),
                enabled: true,
                activate: Box::new(|tray: &mut Self| {
                    tray.sender.send(AppEvent::Control(ControlMsg::Quit));
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}
