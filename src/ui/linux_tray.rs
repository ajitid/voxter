use crate::{AppEvent, AppSender, ControlMsg};
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{Category, Icon, MenuItem, Status, ToolTip, Tray};

pub struct LinuxStatusTray {
    handle: Handle<VoxterLinuxTray>,
}

impl LinuxStatusTray {
    pub fn new(sender: AppSender, has_last_transcript: bool) -> Result<Self, String> {
        let icon = build_ksni_icon();
        let tray = VoxterLinuxTray {
            sender,
            has_last_transcript,
            icon,
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
    icon: Icon,
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

    fn icon_pixmap(&self) -> Vec<Icon> {
        vec![self.icon.clone()]
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

fn build_ksni_icon() -> Icon {
    let rgba_icon = crate::ui::tray_art::build_status_icon_rgba();
    let mut data = rgba_icon.rgba;
    for pixel in data.chunks_exact_mut(4) {
        pixel.rotate_right(1);
    }

    Icon {
        width: rgba_icon.width as i32,
        height: rgba_icon.height as i32,
        data,
    }
}
