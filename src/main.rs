use anyhow::Result;
use eframe::egui;
use obws::{
    requests::inputs::Volume,
    responses::{
        inputs::Input, outputs::Output, scene_collections::SceneCollections, scenes::Scenes,
    },
    Client,
};
use std::{
    net::{IpAddr, SocketAddr},
    thread,
};

fn main() -> Result<()> {
    let (action_tx, mut action_rx) = tokio::sync::mpsc::channel::<Action>(10);
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
                    Action::SetMute(name, val) => {
                        if let Some(obs_client) = &obs_client {
                            obs_client
                                .inputs()
                                .set_muted(&name, val)
                                .await
                                .expect("failed to mute");
                        }
                    }
                    Action::SetVolume(name, value) => {
                        if let Some(obs_client) = &obs_client {
                            let volume = Volume::Mul(value / 100.0);
                            obs_client.inputs().set_volume(&name, volume).await.expect(
                                format!("failed to set volume for device {}", name).as_str(),
                            );
                        }
                    }
                    Action::LogIn(addr, port, pass) => {
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
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "REC",
        native_options,
        Box::new(move |cc| Box::new(App::new(cc, action_tx.clone(), obs_info_rx))),
    )
    .expect("failed to run");

    Ok(())
}

enum Action {
    LogIn(IpAddr, u16, String),
    SetMute(String, bool),
    SetVolume(String, f32),
}

enum ObsInfo {
    InputInfo(Vec<Input>),
    OutputInfo(Vec<Output>),
    SceneInfo(Scenes),
    SceneCollectionInfo(SceneCollections),
}
struct App {
    action_tx: tokio::sync::mpsc::Sender<Action>,
    obs_info_rx: tokio::sync::mpsc::Receiver<ObsInfo>,
    input_info: Vec<Input>,
    output_info: Vec<Output>,
    scene_info: Scenes,
    scene_collection_info: SceneCollections,

    mic_input_name: Option<String>,
    desktop_input_name: Option<String>,

    mic_level: f32,
    desktop_level: f32,
    mic_muted: bool,
    desktop_muted: bool,
    logged_in: bool,

    addr: String,
    port: String,
    pass: String,
}

impl App {
    fn new(
        cc: &eframe::CreationContext<'_>,
        action_tx: tokio::sync::mpsc::Sender<Action>,
        obs_info_rx: tokio::sync::mpsc::Receiver<ObsInfo>,
    ) -> Self {
        Self {
            action_tx,
            obs_info_rx,
            mic_level: 0.0,
            desktop_level: 0.0,
            mic_muted: false,
            desktop_muted: false,
            input_info: Vec::new(),
            output_info: Vec::new(),
            scene_info: Scenes::default(),
            scene_collection_info: SceneCollections::default(),
            mic_input_name: None,
            desktop_input_name: None,
            logged_in: false,
            addr: String::new(),
            port: String::new(),
            pass: String::new(),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if let Ok(obs_info) = self.obs_info_rx.try_recv() {
            match obs_info {
                ObsInfo::InputInfo(input_info) => {
                    self.input_info = input_info;
                }
                ObsInfo::OutputInfo(output_info) => {
                    self.output_info = output_info;
                }
                ObsInfo::SceneInfo(scenes_info) => {
                    self.scene_info = scenes_info;
                }
                ObsInfo::SceneCollectionInfo(collection_info) => {
                    self.scene_collection_info = collection_info;
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("OBS Control");
            // if !self.logged_in {
            //     ui.vertical_centered_justified(|ui| {
            //         ui.add(egui::TextEdit::singleline(&mut self.addr).hint_text("Ip address"));
            //         ui.add(egui::TextEdit::singleline(&mut self.port).hint_text("Port"));
            //         ui.add(egui::TextEdit::singleline(&mut self.pass).hint_text("Password"));
            //         if ui.button("Log In").clicked() {
            //             let addr = self.addr.parse::<IpAddr>().expect("failed to parse ip");
            //             let port = self.port.parse::<u16>().expect("failed to parse port");
            //             self.action_tx
            //                 .try_send(Action::LogIn(addr, port, self.pass.clone()))
            //                 .expect("failed to send login action");
            //             self.logged_in = true;
            //         }
            //     });
            //     let label = egui::Label::new("Not Logged In");
            //     ui.add(label).highlight();
            //     return;
            // }

            if !self.logged_in {
                let address: SocketAddr = "127.0.0.1:4455".parse().expect("failed to parse ip");
                let addr = address.ip();
                let port = address.port();
                self.pass = "test1234".to_string();
                self.action_tx
                    .try_send(Action::LogIn(addr, port, self.pass.clone()))
                    .expect("failed to send login action");
                self.logged_in = true;
            }

            ui.horizontal_top(|ui| {
                egui::Grid::new("Sliders").show(ui, |ui| {
                    ui.vertical_centered_justified(|ui| {
                        egui::ComboBox::from_id_source("mic")
                            .selected_text(
                                self.mic_input_name
                                    .clone()
                                    .unwrap_or("Select Mic".to_string()),
                            )
                            .show_ui(ui, |ui| {
                                for input in &self.input_info {
                                    if !input.kind.contains("input") {
                                        continue;
                                    }

                                    ui.selectable_value(
                                        &mut self.mic_input_name,
                                        Some(input.name.clone()),
                                        input.name.clone(),
                                    );
                                }
                                ui.selectable_value(
                                    &mut self.mic_input_name,
                                    None,
                                    "No Mic".to_string(),
                                );
                            })
                    });
                    ui.vertical_centered_justified(|ui| {
                        egui::ComboBox::from_id_source("desktop")
                            .selected_text(
                                self.desktop_input_name
                                    .clone()
                                    .unwrap_or("Select Desktop".to_string()),
                            )
                            .show_ui(ui, |ui| {
                                for input in &self.input_info {
                                    if !input.kind.contains("output") {
                                        continue;
                                    }

                                    ui.selectable_value(
                                        &mut self.desktop_input_name,
                                        Some(input.name.clone()),
                                        input.name.clone(),
                                    );
                                }
                                ui.selectable_value(
                                    &mut self.desktop_input_name,
                                    None,
                                    "No Desktop".to_string(),
                                );
                            })
                    });
                    ui.end_row();

                    if ui
                        .add(
                            egui::Slider::new(&mut self.mic_level, 0.0..=100.0)
                                .text("Mic Volume")
                                .orientation(egui::SliderOrientation::Vertical)
                                .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 2.0 }),
                        )
                        .dragged()
                    {
                        if let Some(name) = &self.mic_input_name {
                            self.action_tx
                                .try_send(Action::SetVolume(name.clone(), self.mic_level))
                                .expect("failed to send set volume action");
                        }
                    }

                    if ui
                        .add(
                            egui::Slider::new(&mut self.desktop_level, 0.0..=100.0)
                                .text("Desktop Volume")
                                .orientation(egui::SliderOrientation::Vertical)
                                .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 2.0 }),
                        )
                        .dragged()
                    {
                        if let Some(name) = &self.desktop_input_name {
                            self.action_tx
                                .try_send(Action::SetVolume(name.clone(), self.desktop_level))
                                .expect("failed to send set volume action");
                        }
                    }
                    ui.end_row();
                    match self.mic_input_name.clone() {
                        Some(name) => {
                            let mut mic_button: egui::Button = egui::Button::new("Mute Mic");
                            if self.mic_muted {
                                mic_button = egui::Button::new("Unmute Mic");
                                mic_button = mic_button.fill(egui::Color32::RED);
                            }
                            if ui.add(mic_button).clicked() {
                                self.mic_muted = !self.mic_muted;
                                if self.mic_muted {
                                    self.action_tx
                                        .try_send(Action::SetMute(name, true))
                                        .expect("failed to send mute action");
                                } else {
                                    self.action_tx
                                        .try_send(Action::SetMute(name, false))
                                        .expect("failed to send mute action");
                                }
                            }
                        }
                        None => {
                            let label = egui::Label::new("No Mic Selected");
                            ui.add(label).highlight();
                        }
                    }
                    match self.desktop_input_name.clone() {
                        Some(name) => {
                            let mut desktop_button: egui::Button =
                                egui::Button::new("Mute Desktop");
                            if self.desktop_muted {
                                desktop_button = egui::Button::new("Unmute desktop");
                                desktop_button = desktop_button.fill(egui::Color32::RED);
                            }
                            if ui.add(desktop_button).clicked() {
                                self.desktop_muted = !self.desktop_muted;
                                if self.desktop_muted {
                                    self.action_tx
                                        .try_send(Action::SetMute(name, true))
                                        .expect("failed to send mute action");
                                } else {
                                    self.action_tx
                                        .try_send(Action::SetMute(name, false))
                                        .expect("failed to send mute action");
                                }
                            }
                        }
                        None => {
                            let label = egui::Label::new("No Desktop Selected");
                            ui.add(label).highlight();
                        }
                    }

                    ui.end_row();
                });

                egui::Grid::new("All purpose buttons").show(ui, |ui| {
                    for _ in 0..3 {
                        for _ in 0..3 {
                            let mut button = egui::Button::new("Button");
                            button = button.min_size(egui::Vec2::new(100.0, 100.0));
                            ui.add(button);
                        }
                        ui.end_row();
                    }
                });
            });
        });
    }
}
