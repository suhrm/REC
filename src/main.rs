use anyhow::Result;
use obws::{
    requests::inputs::Volume,
    responses::{
        inputs::Input, outputs::Output, scene_collections::SceneCollections, scenes::Scenes,
    },
    Client,
};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    thread,
};

use iced::{widget::container, Application, Element, Renderer, Sandbox};
use iced_widget::{
    button::{self, StyleSheet},
    component, pick_list, text, Component,
};

enum ObsAction {
    SetMute(String, bool),
    SetVolume(String, f32),
    LogIn(IpAddr, u16, String),
}

fn main() -> Result<()> {
    let (action_tx, mut action_rx) = tokio::sync::mpsc::channel::<ObsAction>(10);
    let (obs_info_tx, obs_info_rx) = tokio::sync::mpsc::channel::<ObsInfo>(10);
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build runtime");
        rt.block_on(async {
            let mut obs_client: Option<Client> = None;

            while let Some(action) = action_rx.recv().await {
                match action {
                    ObsAction::SetMute(name, val) => {
                        if let Some(obs_client) = &obs_client {
                            obs_client
                                .inputs()
                                .set_muted(&name, val)
                                .await
                                .expect("failed to mute");
                        }
                    }
                    ObsAction::SetVolume(name, value) => {
                        if let Some(obs_client) = &obs_client {
                            let volume = Volume::Mul(value / 100.0);
                            obs_client.inputs().set_volume(&name, volume).await.expect(
                                format!("failed to set volume for device {}", name).as_str(),
                            );
                        }
                    }
                    ObsAction::LogIn(addr, port, pass) => {
                        let client = Client::connect(addr.to_string(), port, Some(pass))
                            .await
                            .expect("failed to connect to obs");

                        let input_info = client
                            .inputs()
                            .list(None)
                            .await
                            .expect("failed to get input info");
                        let output_info = client
                            .outputs()
                            .list()
                            .await
                            .expect("failed to get output info");

                        let scenes = client
                            .scenes()
                            .list()
                            .await
                            .expect("failed to get scene info");
                        let scene_collections = client
                            .scene_collections()
                            .list()
                            .await
                            .expect("failed to get scene collection info");

                        obs_info_tx
                            .send(ObsInfo::InputInfo(input_info))
                            .await
                            .unwrap();
                        obs_info_tx
                            .send(ObsInfo::OutputInfo(output_info))
                            .await
                            .unwrap();

                        obs_info_tx.send(ObsInfo::SceneInfo(scenes)).await.unwrap();
                        obs_info_tx
                            .send(ObsInfo::SceneCollectionInfo(scene_collections))
                            .await
                            .unwrap();

                        obs_client = Some(client);
                    }
                }
            }
        });
    });
    App::run(iced::Settings::with_flags(AppFlags {
        action_tx,
        obs_info_rx,
    }))?;
    Ok(())
}

struct AppFlags {
    action_tx: tokio::sync::mpsc::Sender<ObsAction>,
    obs_info_rx: tokio::sync::mpsc::Receiver<ObsInfo>,
}

#[derive(Debug, Clone)]
struct SliderState {
    device: Option<String>,
    level: f32,
    muted: bool,
}

impl Default for SliderState {
    fn default() -> Self {
        Self {
            device: None,
            level: 0.0,
            muted: true,
        }
    }
}

struct App {
    app_flags: AppFlags,
    scene_info: Scenes,
    scene_collection_info: SceneCollections,
    sliders: HashMap<String, SliderState>,

    logged_in: bool,

    addr: String,
    port: String,
    pass: String,
}

fn default_sliders() -> HashMap<String, SliderState> {
    let mut default_map = HashMap::new();
    default_map.insert("mic".to_string(), SliderState::default());
    default_map.insert("desktop".to_string(), SliderState::default());
    default_map
}

impl App {
    fn new(app_flags: AppFlags) -> Self {
        App {
            app_flags,
            scene_info: Scenes::default(),
            scene_collection_info: SceneCollections::default(),
            sliders: default_sliders(),
            logged_in: false,
            addr: String::new(),
            port: String::new(),
            pass: String::new(),
        }
    }
    pub fn log_in<'a>(&'a self) -> iced::Element<'a, Action> {
        let mut sliders = iced::widget::row![];
        for (name, attr) in self.sliders.clone() {
            sliders = sliders.push(volume_slider_group(
                name.clone(),
                attr.level,
                attr.muted,
                |msg| Action::VolumeSlider(msg),
                Vec::new(),
            ));
        }
        container(sliders).into()
    }
    pub fn logged_in<'a>(&'a self) -> iced::Element<'a, Action> {
        let mut sliders = iced::widget::row![];
        for (name, attr) in self.sliders.clone() {
            sliders = sliders.push(volume_slider_group(
                name.clone(),
                attr.level,
                attr.muted,
                |msg| Action::VolumeSlider(msg),
                Vec::new(),
            ));
        }
        container(sliders).into()
    }
}

impl Application for App {
    type Executor = iced::executor::Default;

    type Message = Action;

    type Theme = iced::Theme;

    type Flags = AppFlags;

    fn new(flags: Self::Flags) -> (Self, iced::Command<Self::Message>) {
        (App::new(flags), iced::Command::none())
    }

    fn title(&self) -> String {
        String::from("OBS Control")
    }

    fn update(&mut self, message: Self::Message) -> iced::Command<Self::Message> {
        match message {
            Action::LogIn(addr, port, pass) => {
                iced::futures::executor::block_on(
                    self.app_flags
                        .action_tx
                        .send(ObsAction::LogIn(addr, port, pass)),
                )
                .unwrap();
                iced::Command::none()
            }
            Action::VolumeSlider((name, state_change)) => {
                let slider = &mut self
                    .sliders
                    .get_mut(&name)
                    .expect("accesing non existent slider");
                match state_change {
                    VolumeSliderGroupMessage::VolumeChanged(level) => {
                        self.app_flags
                            .action_tx
                            .try_send(ObsAction::SetVolume(name.clone(), level))
                            .expect("Failed to send to obs service");
                        slider.level = level;
                    }

                    VolumeSliderGroupMessage::MuteToggled(muted) => {
                        self.app_flags
                            .action_tx
                            .try_send(ObsAction::SetMute(name.clone(), muted))
                            .expect("Failed to sendto obs service");
                        slider.muted = muted;
                    }

                    VolumeSliderGroupMessage::DeviceSelected(device) => {
                        slider.device = Some(device);
                    }
                }
                iced::Command::none()
            }
        }
    }

    fn view(&self) -> iced::Element<Self::Message> {
        if self.logged_in {
            self.logged_in()
        } else {
            todo!("login system")
        }
    }
}

fn volume_slider_group<Message>(
    name: String,
    level: f32,
    muted: bool,
    on_change: impl Fn(VolumeSliderGroupEvent) -> Message + 'static,
    device_options: Vec<String>,
) -> VolumeSliderGroup<Message> {
    VolumeSliderGroup::new(name, level, muted, on_change, device_options)
}

struct Login {
    password: Option<String>,
    user: Option<String>,
    ip_address: Option<String>,
}

struct VolumeSliderGroup<Message> {
    name: String,
    level: f32,
    muted: bool,
    on_change: Box<dyn Fn(VolumeSliderGroupEvent) -> Message + 'static>,
    selected_device: Option<String>,
    device_options: Vec<String>,
}

impl<Message> VolumeSliderGroup<Message> {
    pub fn new(
        name: String,
        level: f32,
        muted: bool,
        on_change: impl Fn(VolumeSliderGroupEvent) -> Message + 'static,
        device_options: Vec<String>,
    ) -> Self {
        VolumeSliderGroup {
            name,
            level,
            muted,
            on_change: Box::new(on_change),
            selected_device: None,
            device_options,
        }
    }
}

#[derive(Clone, Debug)]
enum VolumeSliderGroupMessage {
    VolumeChanged(f32),
    MuteToggled(bool),
    DeviceSelected(String),
}
type VolumeSliderGroupEvent = (String, VolumeSliderGroupMessage);

impl<Message> Component<Message, Renderer> for VolumeSliderGroup<Message> {
    type State = ();
    type Event = VolumeSliderGroupMessage;

    fn update(&mut self, _state: &mut Self::State, event: Self::Event) -> Option<Message> {
        match event {
            VolumeSliderGroupMessage::VolumeChanged(val) => {
                self.level = val;
                Some((self.on_change)((
                    self.name.clone(),
                    VolumeSliderGroupMessage::VolumeChanged(self.level),
                )))
            }
            VolumeSliderGroupMessage::MuteToggled(muted) => {
                self.muted = !muted;
                Some((self.on_change)((
                    self.name.clone(),
                    VolumeSliderGroupMessage::MuteToggled(self.muted),
                )))
            }
            VolumeSliderGroupMessage::DeviceSelected(device_name) => {
                self.selected_device = Some(device_name);
                Some((self.on_change)((
                    self.name.clone(),
                    VolumeSliderGroupMessage::DeviceSelected(self.selected_device.clone()?),
                )))
            }
        }
    }

    fn view(&self, _state: &Self::State) -> Element<Self::Event, Renderer> {
        dbg!(self.level, self.muted);
        let button = |muted| {
            if muted {
                iced::widget::button("Muted").style(iced::theme::Button::Destructive)
            } else {
                iced::widget::button("Live").style(iced::theme::Button::Positive)
            }
        };

        iced::widget::container(iced::widget::column![
            iced::widget::vertical_slider(
                0.0..=100.0,
                self.level,
                VolumeSliderGroupMessage::VolumeChanged
            ),
            button(self.muted).on_press(VolumeSliderGroupMessage::MuteToggled(self.muted)),
            iced_widget::pick_list(
                self.device_options.clone(),
                self.selected_device.clone(),
                VolumeSliderGroupMessage::DeviceSelected
            )
            .placeholder("Select device")
        ])
        .into()
    }

    fn operate(
        &self,
        _state: &mut Self::State,
        _operation: &mut dyn iced_widget::core::widget::Operation<Message>,
    ) {
    }
}

impl<'a, Message> From<VolumeSliderGroup<Message>> for Element<'a, Message, Renderer>
where
    Message: 'a,
{
    fn from(volume_slider_group: VolumeSliderGroup<Message>) -> Self {
        component(volume_slider_group)
    }
}

#[derive(Debug, Clone)]
enum Action {
    LogIn(IpAddr, u16, String),
    VolumeSlider(VolumeSliderGroupEvent),
}

enum ObsInfo {
    InputInfo(Vec<Input>),
    OutputInfo(Vec<Output>),
    SceneInfo(Scenes),
    SceneCollectionInfo(SceneCollections),
}
