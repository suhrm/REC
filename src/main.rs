use anyhow::{anyhow, Result};
use iced::futures::StreamExt;
use obws::{
    requests::inputs::Volume,
    responses::{
        inputs::Input, outputs::Output, scene_collections::SceneCollections, scenes::Scenes,
    },
    Client,
};
use obws_interface::ActionTx;
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    thread,
};

use iced::{
    futures::FutureExt, widget::container, Application, Element, Renderer, Sandbox, Subscription,
};
use iced_widget::{
    button::{self, StyleSheet},
    component, pick_list, text, Component,
};

mod obws_interface {
    use iced::futures::SinkExt;
    use obws::requests::inputs::Volume;
    use obws::Client;

    use crate::ObsAction;
    use crate::ObsInfo;
    const CHANNEL_SIZE: usize = 10;

    pub type ActionTx = tokio::sync::mpsc::Sender<ObsAction>;
    pub type ActionRx = tokio::sync::mpsc::Receiver<ObsAction>;

    enum State {
        Starting,
        Ready(ActionRx),
    }
    struct Runner {
        obs_client: Option<Client>,
        state: State,
    }
    impl Runner {
        pub fn new() -> Self {
            Self {
                obs_client: None,
                state: State::Starting,
            }
        }
    }
    pub fn subscription() -> iced::Subscription<ObsInfo> {
        iced::subscription::channel(
            std::any::TypeId::of::<Runner>(),
            CHANNEL_SIZE,
            |mut output| async move {
                let mut runner = Runner::new();
                loop {
                    match runner.state {
                        State::Starting => {
                            let (send, recv) =
                                tokio::sync::mpsc::channel::<ObsAction>(CHANNEL_SIZE);
                            output.send(ObsInfo::Ready(send)).await;
                            runner.state = State::Ready(recv);
                        }
                        State::Ready(ref mut recv) => {
                            while let Some(action) = recv.recv().await {
                                match action {
                                    ObsAction::SetMute(name, val) => {
                                        if let Some(obs_client) = &runner.obs_client {
                                            obs_client
                                                .inputs()
                                                .set_muted(&name, val)
                                                .await
                                                .expect("failed to mute");
                                        }
                                    }
                                    ObsAction::SetVolume(name, value) => {
                                        if let Some(obs_client) = &runner.obs_client {
                                            let volume = Volume::Mul(value / 100.0);
                                            obs_client
                                                .inputs()
                                                .set_volume(&name, volume)
                                                .await
                                                .expect(
                                                    format!(
                                                        "failed to set volume for device {}",
                                                        name
                                                    )
                                                    .as_str(),
                                                );
                                        }
                                    }
                                    ObsAction::LogIn(addr, pass) => {
                                        dbg!(&addr, &pass);
                                        let client = Client::connect(
                                            addr.ip().to_string(),
                                            addr.port(),
                                            Some(pass),
                                        )
                                        .await
                                        .expect("failed to connect to obs");
                                        output.send(ObsInfo::LoggedIn).await.unwrap();

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

                                        output.send(ObsInfo::InputInfo(input_info)).await.unwrap();
                                        output
                                            .send(ObsInfo::OutputInfo(output_info))
                                            .await
                                            .unwrap();

                                        output.send(ObsInfo::SceneInfo(scenes)).await.unwrap();
                                        output
                                            .send(ObsInfo::SceneCollectionInfo(scene_collections))
                                            .await
                                            .unwrap();

                                        runner.obs_client = Some(client);
                                    }
                                }
                            }
                        }
                    }
                }
            },
        )
    }
}

enum ObsAction {
    SetMute(String, bool),
    SetVolume(String, f32),
    LogIn(SocketAddr, String),
}

fn main() -> Result<()> {
    App::run(iced::Settings::with_flags(()))?;
    Ok(())
}

enum Views {
    Login,
    LoggedIn,
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
    scene_info: Scenes,
    scene_collection_info: SceneCollections,
    sliders: HashMap<String, SliderState>,
    obs_action_tx: Option<ActionTx>,

    addr: String,
    port: String,
    pass: String,
    current_view: Views,
}

fn default_sliders() -> HashMap<String, SliderState> {
    let mut default_map = HashMap::new();
    default_map.insert("mic".to_string(), SliderState::default());
    default_map.insert("desktop".to_string(), SliderState::default());
    default_map
}

impl App {
    fn new(app_flags: ()) -> Self {
        App {
            scene_info: Scenes::default(),
            scene_collection_info: SceneCollections::default(),
            sliders: HashMap::new(),
            obs_action_tx: None,
            addr: String::new(),
            port: String::new(),
            pass: String::new(),
            current_view: Views::Login,
        }
    }
    pub fn logged_in<'a>(&'a self) -> iced::Element<'a, Action> {
        let mut sliders = iced::widget::row![];
        for (name, attr) in self.sliders.clone() {
            sliders = sliders.push(volume_slider_group(
                |msg| Action::VolumeSlider(msg),
                self.sliders.clone().into_keys().collect(),
            ));
        }
        container(sliders).into()
    }
    pub fn not_logged_in<'a>(&'a self) -> iced::Element<'a, Action> {
        let login = iced::widget::row![login_window(|msg| Action::LogIn(msg))];
        container(login).into()
    }
    fn send_obs_action(&self, action: ObsAction) -> iced::Command<Action> {
        let tx = self.obs_action_tx.clone().unwrap();
        let send =
            (|tx: tokio::sync::mpsc::Sender<ObsAction>| async move { tx.send(action).await });
        iced::Command::perform(send(tx), |err| match err {
            Ok(_) => Action::Err(None),
            Err(e) => Action::Err(Some(anyhow!(e))),
        })
    }
}

impl Application for App {
    type Executor = iced::executor::Default;

    type Message = Action;

    type Theme = iced::Theme;

    type Flags = ();

    fn new(flags: Self::Flags) -> (Self, iced::Command<Self::Message>) {
        (App::new(flags), iced::Command::none())
    }

    fn title(&self) -> String {
        String::from("OBS Control")
    }
    fn subscription(&self) -> iced::Subscription<Self::Message> {
        obws_interface::subscription().map(Action::ObsInfo)
    }

    fn update(&mut self, message: Self::Message) -> iced::Command<Self::Message> {
        match message {
            Action::LogIn(Credentials { sock_addr, pass }) => {
                self.send_obs_action(ObsAction::LogIn(sock_addr.unwrap(), pass))
            }
            Action::VolumeSlider((name, state_change)) => {
                let slider = &mut self
                    .sliders
                    .get_mut(&name)
                    .expect("accesing non existent slider");
                match state_change {
                    VolumeSliderGroupMessage::VolumeChanged(level) => {
                        slider.level = level;
                    }

                    VolumeSliderGroupMessage::MuteToggled(muted) => {
                        self.obs_action_tx
                            .as_ref()
                            .expect("send must be some at this point")
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
            Action::ObsInfo(info) => {
                dbg!(&info);
                match info {
                    ObsInfo::LoggedIn => self.current_view = Views::LoggedIn,
                    ObsInfo::InputInfo(inputs) => {
                        for input in inputs.iter() {
                            self.sliders
                                .insert(input.name.clone(), SliderState::default());
                        }
                    }
                    ObsInfo::Ready(obs_action_tx) => self.obs_action_tx = Some(obs_action_tx),
                    _ => (),
                }
                iced::Command::none()
            }
            Action::Err(e) => match e {
                Some(err) => panic!("{}", err),
                None => iced::Command::none(),
            },
        }
    }

    fn view(&self) -> iced::Element<Self::Message> {
        match self.current_view {
            Views::Login => self.not_logged_in(),
            Views::LoggedIn => self.logged_in(),
        }
    }
}

fn volume_slider_group<Message>(
    on_change: impl Fn(VolumeSliderGroupEvent) -> Message + 'static,
    device_options: Vec<String>,
) -> VolumeSliderGroup<Message> {
    VolumeSliderGroup::new(on_change, device_options)
}

struct Login<Message> {
    password: String,
    ip_address: String,
    on_change: Box<dyn Fn(Credentials) -> Message + 'static>,
}

fn login_window<Message>(on_change: impl Fn(Credentials) -> Message + 'static) -> Login<Message> {
    Login::new(on_change)
}

impl<Message> Login<Message> {
    pub fn new(on_change: impl Fn(Credentials) -> Message + 'static) -> Self {
        Self {
            password: String::new(),
            ip_address: String::new(),
            on_change: Box::new(on_change),
        }
    }
}
#[derive(Default, Debug, Clone)]
struct Credentials {
    sock_addr: Option<SocketAddr>,
    pass: String,
}
#[derive(Debug, Clone)]
enum LoginEvent {
    Password(String),
    IpAddress(String),
    Submit,
}

impl<Message> Component<Message, Renderer> for Login<Message> {
    type State = Credentials;

    type Event = LoginEvent;

    fn update(&mut self, state: &mut Self::State, event: Self::Event) -> Option<Message> {
        match event {
            LoginEvent::Password(pass) => {
                self.password = pass;
                None
            }
            LoginEvent::IpAddress(ip_addr) => {
                self.ip_address = ip_addr;
                None
            }
            LoginEvent::Submit => {
                self.password = "test1234".to_string();
                self.ip_address = "127.0.0.1:4455".to_string();
                if let Ok(addr) = self.ip_address.as_str().parse() {
                    Some((self.on_change)(Credentials {
                        sock_addr: Some(addr),
                        pass: self.password.clone(),
                    }))
                } else {
                    None
                }
            }
        }
    }

    fn view(&self, _state: &Self::State) -> iced_widget::core::Element<'_, Self::Event, Renderer> {
        iced::widget::container(iced::widget::column![
            iced_widget::text_input("IP-address", self.ip_address.as_str())
                .on_input(Self::Event::IpAddress),
            iced_widget::text_input("Password", self.password.as_str())
                .on_input(Self::Event::Password),
            iced_widget::button("Login").on_press(Self::Event::Submit)
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
impl<'a, Message> From<Login<Message>> for Element<'a, Message, Renderer>
where
    Message: 'a,
{
    fn from(login: Login<Message>) -> Self {
        component(login)
    }
}

struct VolumeSliderGroup<Message> {
    on_change: Box<dyn Fn(VolumeSliderGroupEvent) -> Message + 'static>,
    device_options: Vec<String>,
}

impl<Message> VolumeSliderGroup<Message> {
    pub fn new(
        on_change: impl Fn(VolumeSliderGroupEvent) -> Message + 'static,
        device_options: Vec<String>,
    ) -> Self {
        VolumeSliderGroup {
            on_change: Box::new(on_change),
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
    type State = SliderState;
    type Event = VolumeSliderGroupMessage;

    fn update(&mut self, state: &mut Self::State, event: Self::Event) -> Option<Message> {
        dbg!(&state);
        if let VolumeSliderGroupMessage::DeviceSelected(name) = event {
            state.device = Some(name);
            return Some((self.on_change)((
                state.device.clone().unwrap(),
                VolumeSliderGroupMessage::DeviceSelected(state.device.clone()?),
            )));
        }

        if state.device.is_some() {
            return match event {
                VolumeSliderGroupMessage::VolumeChanged(val) => {
                    state.level = val;
                    Some((self.on_change)((
                        state.device.clone().unwrap(),
                        VolumeSliderGroupMessage::VolumeChanged(state.level),
                    )))
                }
                VolumeSliderGroupMessage::MuteToggled(muted) => {
                    state.muted = !muted;
                    Some((self.on_change)((
                        state.device.clone().unwrap(),
                        VolumeSliderGroupMessage::MuteToggled(state.muted),
                    )))
                }
                _ => unreachable!(),
            };
        }
        None
    }

    fn view(&self, state: &Self::State) -> Element<Self::Event, Renderer> {
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
                state.level,
                VolumeSliderGroupMessage::VolumeChanged
            ),
            button(state.muted).on_press(VolumeSliderGroupMessage::MuteToggled(state.muted)),
            iced::widget::pick_list(
                self.device_options.clone(),
                state.device.clone(),
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

#[derive(Debug)]
enum Action {
    LogIn(Credentials),
    VolumeSlider(VolumeSliderGroupEvent),
    ObsInfo(ObsInfo),
    Err(Option<anyhow::Error>),
}

#[derive(Debug, Clone)]
enum ObsInfo {
    LoggedIn,
    Ready(obws_interface::ActionTx),

    InputInfo(Vec<Input>),
    OutputInfo(Vec<Output>),
    SceneInfo(Scenes),
    SceneCollectionInfo(SceneCollections),
}
