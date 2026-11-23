mod apply;

use std::{collections::BTreeMap, env, fmt};

use anyhow::{bail, ensure, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use iced::{
    widget::{button, checkbox, column, container, pick_list, row, scrollable, text, text_input},
    Alignment, Element, Font,
    Length::{Fill, FillPortion},
    Task,
};
use skillnet::{
    models::{self, Model, Provider},
    templates::{self, BindingsDoc, Fallback, Route, Seat, Settings, SkillBinding, TemplateMeta},
};

use apply::{Applied, Apply};

type Models = BTreeMap<String, Vec<(String, Model)>>;

fn canonical_model_name(model: &Model) -> String {
    if model.sub_provider.as_deref() == Some("Claude Agent") && !model.name.starts_with("Claude ") {
        format!("Claude {}", model.name)
    } else {
        model.name.clone()
    }
}

fn group_models(providers: Vec<Provider>) -> Models {
    // ponytail: display-name equality is a heuristic; use an explicit alias table if it fails.
    let mut groups = Models::new();
    for provider in providers {
        for model in provider.models {
            groups
                .entry(canonical_model_name(&model))
                .or_default()
                .push((provider.instance_id.clone(), model));
        }
    }
    groups
}

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let explicit = match args.next() {
        None => env::var("SKILLNET_CONFIG")
            .ok()
            .filter(|path| !path.trim().is_empty()),
        Some(arg) if arg == "--config" => Some(args.next().context("--config needs a path")?),
        Some(arg) if arg.starts_with("--config=") => Some(arg["--config=".len()..].to_owned()),
        Some(_) => bail!("usage: skillnet-ui [--config PATH]"),
    }
    .map(Utf8PathBuf::from);
    ensure!(args.next().is_none(), "usage: skillnet-ui [--config PATH]");
    let config = match &explicit {
        Some(path) => path.clone(),
        None => {
            let config_home = match env::var("XDG_CONFIG_HOME").ok() {
                Some(path) if !path.trim().is_empty() => Utf8PathBuf::from(path.trim()),
                _ => {
                    Utf8PathBuf::from(env::var("HOME").context("HOME is not set")?).join(".config")
                }
            };
            config_home.join("skillnet/skillnet.toml")
        }
    };
    iced::application(move || App::load(&config), App::update, App::view)
        .title("Skillnet — Model bindings")
        .window(iced::window::Settings {
            size: iced::Size::new(1060.0, 850.0),
            min_size: Some(iced::Size::new(760.0, 590.0)),
            position: iced::window::Position::Centered,
            ..Default::default()
        })
        .run()?;
    Ok(())
}

struct App {
    config: Utf8PathBuf,
    settings: Settings,
    fleet_writable: bool,
    templates: Vec<(String, TemplateMeta)>,
    fleet: BindingsDoc,
    local: BindingsDoc,
    models: Models,
    model_filter: String,
    selected: Option<usize>,
    drafts: BTreeMap<String, SkillBinding>,
    /// Skills whose draft the user changed; only these survive an Apply.
    edited: std::collections::BTreeSet<String>,
    preview: Result<String>,
    menu_open: bool,
    busy: bool,
    status: String,
    hosts: Option<String>,
}

#[derive(Clone)]
enum Message {
    Select(usize),
    RefreshModels,
    FilterModels(String),
    AddSeat,
    RemoveSeat(usize),
    EditSeat(usize, SeatEdit),
    ToggleMenu,
    Apply(Apply),
    Applied(Box<Applied>),
    Hosts(Result<String, String>),
}

#[derive(Clone)]
enum SeatEdit {
    Name(String),
    Label(String),
    Model(String),
    Route(Option<usize>, Route),
    Option(Option<usize>, String, String),
    AddFallback(Route),
    EnableFallback(usize, bool),
    MoveFallback(usize, usize),
    RemoveFallback(usize),
}

impl App {
    fn load(config: &Utf8Path) -> (Result<Self>, Task<Message>) {
        let init = (|| -> Result<Self> {
            let settings = Settings::load(config)?;
            let fleet = templates::load_existing(settings.fleet.as_slice())?;
            let local = templates::load_existing(settings.local.as_slice())?;
            let templates = templates::list_templates(&settings.canonical)?;
            let fleet_writable = settings
                .fleet
                .as_deref()
                .is_some_and(apply::is_writable_target);
            let mut app = Self {
                config: config.to_owned(),
                settings,
                fleet_writable,
                templates,
                fleet,
                local,
                models: Models::new(),
                model_filter: String::new(),
                selected: None,
                drafts: BTreeMap::new(),
                edited: Default::default(),
                preview: Ok(String::new()),
                menu_open: false,
                busy: false,
                status: "Ready".into(),
                hosts: None,
            };
            app.refresh_models();
            if !app.templates.is_empty() {
                app.select(0);
                app.render_preview();
            }
            Ok(app)
        })();
        let task = init
            .as_ref()
            .ok()
            .and_then(|app| app.hosts_command().cloned())
            .map_or_else(Task::none, |command| {
                Task::perform(async move { apply::hosts(&command) }, Message::Hosts)
            });
        (init, task)
    }

    fn hosts_command(&self) -> Option<&Vec<String>> {
        self.local
            .ui
            .as_ref()
            .or(self.fleet.ui.as_ref())
            .and_then(|ui| ui.hosts_command.as_ref())
    }

    fn refresh_models(&mut self) {
        match models::load_t3_caches(&self.settings.t3_caches) {
            Ok(mut providers) => {
                if let Some(allowlist) = self
                    .local
                    .ui
                    .as_ref()
                    .or(self.fleet.ui.as_ref())
                    .and_then(|ui| ui.providers.as_ref())
                {
                    providers.retain(|provider| allowlist.contains(&provider.instance_id));
                }
                self.status = format!("Loaded {} model providers", providers.len());
                self.models = group_models(providers);
                for draft in self.drafts.values_mut() {
                    fill_defaults(&self.models, draft);
                }
            }
            Err(error) => self.status = format!("{error:#}"),
        }
    }

    fn select(&mut self, index: usize) {
        self.selected = Some(index);
        self.menu_open = false;
        let (skill, _) = &self.templates[index];
        if !self.drafts.contains_key(skill) {
            let draft = self.fresh_draft(skill);
            self.drafts.insert(skill.clone(), draft);
        }
    }

    /// The draft a skill starts from: its local, else fleet, binding on disk.
    fn fresh_draft(&self, skill: &str) -> SkillBinding {
        let mut draft = self
            .local
            .skills
            .get(skill)
            .or_else(|| self.fleet.skills.get(skill))
            .cloned()
            .unwrap_or(SkillBinding { seats: Vec::new() });
        fill_defaults(&self.models, &mut draft);
        for seat in &mut draft.seats {
            if seat.label.is_some() && seat.label == auto_label(&self.models, &seat.route) {
                seat.label = None;
            }
        }
        draft
    }

    fn rendered_binding(&self, skill: &str) -> SkillBinding {
        let mut binding = self.drafts[skill].clone();
        for seat in &mut binding.seats {
            seat.label = seat
                .label
                .as_deref()
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .map(str::to_owned)
                .or_else(|| auto_label(&self.models, &seat.route));
        }
        binding
    }

    fn render_preview(&mut self) {
        if let Some(index) = self.selected {
            let (skill, _) = &self.templates[index];
            let binding = self.rendered_binding(skill);
            self.preview = if binding
                .seats
                .iter()
                .any(|seat| seat.route.model.is_empty() || seat.route.provider.is_empty())
            {
                Err(anyhow::anyhow!(
                    "Choose a model and provider for each seat."
                ))
            } else {
                templates::render_skill(&self.settings.canonical, skill, &binding)
            };
        }
    }

    fn update(state: &mut Result<Self>, message: Message) -> Task<Message> {
        let Ok(app) = state else {
            return Task::none();
        };
        match message {
            Message::Hosts(result) => match result {
                Ok(hosts) => app.hosts = Some(hosts),
                Err(error) => app.status = error,
            },
            Message::Applied(result) => {
                let Applied {
                    fleet,
                    local,
                    status,
                    hosts,
                    dropped,
                    saved,
                } = *result;
                // Only a fully completed Apply saves the selected skill's edits.
                if saved {
                    if let Some(index) = app.selected {
                        app.edited.remove(&app.templates[index].0);
                    }
                }
                // Unedited drafts reload from the refreshed files; edits stay.
                let edited = &app.edited;
                app.drafts.retain(|skill, _| edited.contains(skill));
                if let Some(fleet) = fleet {
                    app.fleet = fleet;
                }
                if let Some(local) = local {
                    app.local = local;
                }
                if let Some(hosts) = hosts {
                    app.hosts = Some(hosts);
                }
                if let Some(index) = app.selected {
                    if dropped {
                        app.drafts.remove(&app.templates[index].0);
                    }
                    // Recreates the selected draft if it was reloaded above.
                    app.select(index);
                }
                app.fleet_writable = app
                    .settings
                    .fleet
                    .as_deref()
                    .is_some_and(apply::is_writable_target);
                app.status = status;
                app.busy = false;
            }
            _ if app.busy => {}
            Message::Select(index) => app.select(index),
            Message::RefreshModels => app.refresh_models(),
            Message::FilterModels(filter) => app.model_filter = filter,
            Message::ToggleMenu => app.menu_open = !app.menu_open,
            message => {
                let Some(index) = app.selected else {
                    return Task::none();
                };
                let (skill, meta) = &app.templates[index];
                if let Message::Apply(scope) = message {
                    let binding = app.rendered_binding(skill);
                    let opened = (
                        app.settings.fleet.clone(),
                        app.settings.local.clone(),
                        app.settings.canonical.clone(),
                    );
                    let config = app.config.clone();
                    let hosts_command = app.hosts_command().cloned();
                    let skill = skill.clone();
                    app.busy = true;
                    app.menu_open = false;
                    app.status = if scope == Apply::Fleet {
                        "Applying fleet-wide… (approve GPG signing if prompted)".into()
                    } else {
                        "Applying…".into()
                    };
                    return Task::perform(
                        async move {
                            apply::run(scope, &config, &opened, &skill, &binding, hosts_command)
                        },
                        |result| Message::Applied(Box::new(result)),
                    );
                }
                let seats = &mut app.drafts.get_mut(skill).expect("selected draft").seats;
                let before = seats.clone();
                match message {
                    Message::AddSeat if meta.max.is_none_or(|max| seats.len() < max) => {
                        seats.push(Seat {
                            name: String::new(),
                            label: None,
                            route: Route {
                                provider: String::new(),
                                model: String::new(),
                                options: BTreeMap::new(),
                            },
                            fallbacks: Vec::new(),
                        });
                    }
                    Message::RemoveSeat(i) if seats.len() > meta.min && i < seats.len() => {
                        seats.remove(i);
                    }
                    Message::EditSeat(i, edit) => {
                        if let Some(seat) = seats.get_mut(i) {
                            edit_seat(seat, edit, &app.models);
                        }
                    }
                    _ => {}
                }
                // Only an action that changed the draft marks it edited.
                if *seats != before {
                    app.edited.insert(skill.clone());
                }
            }
        }
        app.render_preview();
        Task::none()
    }

    fn view(state: &Result<Self>) -> Element<'_, Message> {
        let app = match state {
            Ok(app) => app,
            Err(error) => return container(text(format!("{error:#}"))).padding(16).into(),
        };
        let mut skills = column![text("Templated skills").size(20)].spacing(8);
        for (i, (skill, _)) in app.templates.iter().enumerate() {
            let skill_button = button(text(skill))
                .width(Fill)
                .style(if app.selected == Some(i) {
                    button::primary
                } else {
                    button::secondary
                })
                .on_press_maybe((!app.busy).then_some(Message::Select(i)));
            skills = skills.push(skill_button);
            if app.local.skills.contains_key(skill) {
                skills = skills.push(
                    container(text("local override").size(12))
                        .style(container::rounded_box)
                        .padding([2, 6]),
                );
            }
        }
        // Status and fleet live in the sidebar so the editor keeps the full height.
        let mut sidebar = column![
            scrollable(skills).height(Fill),
            scrollable(text(&app.status).size(13)).height(96),
        ]
        .spacing(12)
        .width(240);
        if app.hosts_command().is_some() {
            // ponytail: no applied/behind status; query the deployed revision per host to add it.
            sidebar = sidebar
                .push(text("Fleet").size(17))
                .push(
                    scrollable(
                        text(app.hosts.as_deref().unwrap_or("No host information")).size(13),
                    )
                    .height(120),
                )
                .push(
                    text("Fleet-wide changes take effect when each host next runs global sync.")
                        .size(13),
                );
        }
        row![sidebar, app.editor()].spacing(16).padding(16).into()
    }

    fn editor(&self) -> Element<'_, Message> {
        let refresh = button("Refresh models")
            .style(button::secondary)
            .on_press_maybe((!self.busy).then_some(Message::RefreshModels));
        let Some(index) = self.selected else {
            return column![refresh, text("No templated skills found.")]
                .spacing(12)
                .width(Fill)
                .into();
        };
        let (skill, meta) = &self.templates[index];
        let seats = &self.drafts[skill].seats;
        let mut editor = column![text("Seats").size(19)].spacing(12);
        for (i, seat) in seats.iter().enumerate() {
            editor = editor.push(self.seat_view(i, seat, seats.len() > meta.min));
        }
        let can_add = !self.busy && meta.max.is_none_or(|max| seats.len() < max);
        editor =
            editor.push(button("Add seat").on_press_maybe(can_add.then_some(Message::AddSeat)));
        let preview = match &self.preview {
            Ok(content) => text(content).font(Font::MONOSPACE),
            Err(error) => text(format!("{error:#}")),
        };
        let can_apply = !self.busy && self.preview.is_ok();
        let fleet_writable = self.fleet_writable;
        let has_local_override = self.local.skills.contains_key(skill);
        let mut actions = if fleet_writable {
            row![
                button("Apply fleet-wide")
                    .on_press_maybe(can_apply.then_some(Message::Apply(Apply::Fleet))),
                button(if self.menu_open { "▴" } else { "▾" })
                    .style(button::secondary)
                    .on_press_maybe((!self.busy).then_some(Message::ToggleMenu)),
            ]
        } else {
            row![
                button("Apply local only").on_press_maybe(
                    (can_apply && self.settings.local.is_some())
                        .then_some(Message::Apply(Apply::Local))
                ),
                button("Apply fleet-wide").style(button::secondary),
            ]
            .spacing(8)
        };
        if has_local_override {
            actions = actions.push(
                button("Drop local override")
                    .style(button::secondary)
                    .on_press_maybe(
                        // Dropping needs a fleet binding to fall back to.
                        (!self.busy
                            && self.settings.local.is_some()
                            && self.fleet.skills.contains_key(skill))
                        .then_some(Message::Apply(Apply::Drop)),
                    ),
            );
        }
        let filter_input = text_input("Filter models…", &self.model_filter)
            .on_input(Message::FilterModels)
            .width(220);
        let mut panel = column![
            row![text(skill).size(24).width(Fill), filter_input, refresh]
                .spacing(8)
                .align_y(Alignment::Center),
            row![
                scrollable(container(editor).padding([0, 8]))
                    .height(Fill)
                    .width(FillPortion(3)),
                column![
                    text("SKILL.md preview").size(19),
                    container(scrollable(preview).height(Fill).width(Fill))
                        .style(container::rounded_box)
                        .padding(8),
                ]
                .spacing(8)
                .width(FillPortion(2)),
            ]
            .spacing(16)
            .height(Fill),
        ]
        .spacing(8)
        .width(Fill);
        if fleet_writable && self.menu_open {
            panel = panel.push(
                button("Apply local only").on_press_maybe(
                    (can_apply && self.settings.local.is_some())
                        .then_some(Message::Apply(Apply::Local)),
                ),
            );
        }
        if self.settings.fleet.is_some() && !fleet_writable {
            panel = panel.push(
                text("Fleet bindings are read-only on this host; changes apply locally.").size(13),
            );
        } else if fleet_writable && has_local_override {
            panel = panel.push(
                text("Applying fleet-wide will also remove this skill's local override.").size(13),
            );
        }
        panel.push(actions).into()
    }

    fn seat_view<'a>(&'a self, i: usize, seat: &'a Seat, can_remove: bool) -> Element<'a, Message> {
        let send = move |edit| Message::EditSeat(i, edit);
        let group = find_model_group(&self.models, &seat.route).map(|(name, _)| name);
        let auto = auto_label(&self.models, &seat.route);
        let query = self.model_filter.trim().to_ascii_lowercase();
        let model_names: Vec<&str> = self
            .models
            .keys()
            .filter(|name| {
                query.is_empty()
                    || name.to_ascii_lowercase().contains(&query)
                    || group == Some(name.as_str())
            })
            .map(String::as_str)
            .collect();
        let mut fields = column![
            row![
                text(format!("Seat {}", i + 1)).width(Fill),
                button("Remove seat")
                    .style(button::secondary)
                    .on_press_maybe((can_remove && !self.busy).then_some(Message::RemoveSeat(i))),
            ]
            .align_y(Alignment::Center),
            row![
                column![
                    text("Name"),
                    text_input("Seat name", &seat.name).on_input(move |v| send(SeatEdit::Name(v)))
                ],
                column![
                    text("Label"),
                    text_input(
                        auto.as_deref().unwrap_or("Automatic label"),
                        seat.label.as_deref().unwrap_or(""),
                    )
                    .on_input(move |v| send(SeatEdit::Label(v)))
                ],
            ]
            .spacing(8),
            row![
                pick_list(model_names, group, move |name| send(SeatEdit::Model(
                    name.into()
                )),)
                .placeholder(if seat.route.model.is_empty() {
                    "Model"
                } else {
                    &seat.route.model
                })
                .width(Fill),
                self.provider_picker(i, group, Some(&seat.route), None, |_, _| true),
            ]
            .spacing(8),
            self.option_pickers(i, None, &seat.route),
            text("Fallbacks").size(15),
        ]
        .spacing(8);

        for (j, fallback) in seat.fallbacks.iter().enumerate() {
            fields = fields.push(
                column![
                    row![
                        self.provider_picker(
                            i,
                            group,
                            Some(&fallback.route),
                            Some(j),
                            |provider, slug| {
                                !same_route(&seat.route, provider, slug)
                                    && !seat.fallbacks.iter().enumerate().any(|(k, entry)| {
                                        k != j && same_route(&entry.route, provider, slug)
                                    })
                            }
                        ),
                        checkbox(fallback.enabled)
                            .label("Enabled")
                            .on_toggle(move |on| send(SeatEdit::EnableFallback(j, on))),
                        button("Up").on_press_maybe(
                            j.checked_sub(1)
                                .map(|to| send(SeatEdit::MoveFallback(j, to)))
                        ),
                        button("Down").on_press_maybe(
                            (j + 1 < seat.fallbacks.len())
                                .then_some(send(SeatEdit::MoveFallback(j, j + 1)))
                        ),
                        button("Remove").on_press(send(SeatEdit::RemoveFallback(j))),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                    self.option_pickers(i, Some(j), &fallback.route),
                ]
                .spacing(6),
            );
        }
        fields = fields.push(
            self.provider_picker(i, group, None, None, |provider, slug| {
                !same_route(&seat.route, provider, slug)
                    && !seat
                        .fallbacks
                        .iter()
                        .any(|entry| same_route(&entry.route, provider, slug))
            }),
        );
        container(fields)
            .style(container::rounded_box)
            .padding(10)
            .width(Fill)
            .into()
    }

    fn provider_picker<'a>(
        &'a self,
        i: usize,
        group: Option<&str>,
        selected: Option<&Route>,
        fallback: Option<usize>,
        include: impl Fn(&str, &str) -> bool,
    ) -> Element<'a, Message> {
        let routes = group.map_or(&[][..], |name| self.models[name].as_slice());
        let choices = routes
            .iter()
            .filter(|(provider, model)| include(provider, &model.slug))
            .map(|(provider, model)| RouteChoice {
                label: route_label(provider, model, routes),
                route: new_route(provider, model),
            })
            .collect::<Vec<_>>();
        let choice = selected.and_then(|route| {
            choices
                .iter()
                .find(|choice| {
                    choice.route.provider == route.provider && choice.route.model == route.model
                })
                .cloned()
        });
        let placeholder = selected.map_or("Add fallback", |route| {
            if route.provider.is_empty() {
                "Provider"
            } else {
                &route.provider
            }
        });
        let replacing = selected.is_some();
        pick_list(choices, choice, move |choice: RouteChoice| {
            Message::EditSeat(
                i,
                if replacing {
                    SeatEdit::Route(fallback, choice.route)
                } else {
                    SeatEdit::AddFallback(choice.route)
                },
            )
        })
        .placeholder(placeholder)
        .width(Fill)
        .into()
    }

    fn option_pickers<'a>(
        &'a self,
        i: usize,
        fallback: Option<usize>,
        route: &'a Route,
    ) -> Element<'a, Message> {
        let mut fields = row![].spacing(8);
        if let Some(model) = find_model(&self.models, route) {
            for descriptor in &model.capabilities.option_descriptors {
                if descriptor.kind != "select" {
                    continue;
                }
                fields = fields.push(
                    column![
                        text(&descriptor.label).size(13),
                        pick_list(
                            descriptor
                                .options
                                .iter()
                                .map(|option| option.id.as_str())
                                .collect::<Vec<_>>(),
                            route.options.get(&descriptor.id).map(String::as_str),
                            move |value| Message::EditSeat(
                                i,
                                SeatEdit::Option(fallback, descriptor.id.clone(), value.into())
                            ),
                        )
                        .placeholder("Select value"),
                    ]
                    .spacing(4),
                );
            }
        }
        fields.wrap().into()
    }
}

#[derive(Clone, PartialEq)]
struct RouteChoice {
    label: String,
    route: Route,
}

impl fmt::Display for RouteChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

fn same_route(route: &Route, provider: &str, slug: &str) -> bool {
    route.provider == provider && route.model == slug
}

fn route_label(provider: &str, model: &Model, routes: &[(String, Model)]) -> String {
    if let Some(sub) = model
        .sub_provider
        .as_deref()
        .map(str::trim)
        .filter(|sub| !sub.is_empty())
    {
        let duplicate_sub = routes
            .iter()
            .filter(|(id, entry)| {
                id == provider && entry.sub_provider.as_deref().map(str::trim) == Some(sub)
            })
            .nth(1)
            .is_some();
        if duplicate_sub {
            format!("{provider} · {sub} ({})", model.slug)
        } else {
            format!("{provider} · {sub}")
        }
    } else if routes
        .iter()
        .filter(|(id, _)| id == provider)
        .nth(1)
        .is_some()
    {
        format!("{provider} ({})", model.slug)
    } else {
        provider.to_owned()
    }
}

fn find_model_group<'a>(groups: &'a Models, route: &Route) -> Option<(&'a str, &'a Model)> {
    groups.iter().find_map(|(name, routes)| {
        routes.iter().find_map(|(provider, model)| {
            same_route(route, provider, &model.slug).then_some((name.as_str(), model))
        })
    })
}

fn find_model<'a>(groups: &'a Models, route: &Route) -> Option<&'a Model> {
    find_model_group(groups, route).map(|(_, model)| model)
}

fn new_route(provider: &str, model: &Model) -> Route {
    let mut route = Route {
        provider: provider.into(),
        model: model.slug.clone(),
        options: BTreeMap::new(),
    };
    default_options(model, &mut route);
    route
}

fn default_options(model: &Model, route: &mut Route) {
    for descriptor in &model.capabilities.option_descriptors {
        if descriptor.kind == "select" {
            let default = descriptor
                .options
                .iter()
                .find(|value| value.is_default == Some(true))
                .map(|value| value.id.as_str())
                .or_else(|| {
                    descriptor
                        .current_value
                        .as_ref()
                        .and_then(serde_json::Value::as_str)
                        .filter(|current| {
                            descriptor
                                .options
                                .iter()
                                .any(|option| option.id == *current)
                        })
                });
            if let Some(value) = default {
                route
                    .options
                    .entry(descriptor.id.clone())
                    .or_insert_with(|| value.to_owned());
            }
        }
    }
}

fn fill_defaults(models: &Models, binding: &mut SkillBinding) {
    for seat in &mut binding.seats {
        for route in std::iter::once(&mut seat.route)
            .chain(seat.fallbacks.iter_mut().map(|entry| &mut entry.route))
        {
            if let Some(model) = find_model(models, route) {
                default_options(model, route);
            }
        }
    }
}

fn auto_label(models: &Models, route: &Route) -> Option<String> {
    let (name, _) = find_model_group(models, route)?;
    Some(
        std::iter::once(name)
            .chain(route.options.values().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn edit_seat(seat: &mut Seat, edit: SeatEdit, models: &Models) {
    match edit {
        SeatEdit::Name(name) => seat.name = name,
        SeatEdit::Label(label) => {
            // Keep the raw text while typing; only an empty field means automatic.
            seat.label = (!label.is_empty()).then_some(label);
        }
        SeatEdit::Model(name) => {
            if find_model_group(models, &seat.route).is_none_or(|(group, _)| group != name) {
                if let Some((provider, model)) = models.get(&name).and_then(|routes| routes.first())
                {
                    seat.route = new_route(provider, model);
                    seat.fallbacks.clear();
                }
            }
        }
        SeatEdit::Route(None, route) => {
            if seat.route.provider != route.provider || seat.route.model != route.model {
                seat.fallbacks
                    .retain(|entry| !same_route(&entry.route, &route.provider, &route.model));
                seat.route = route;
            }
        }
        SeatEdit::Route(Some(i), route) => {
            if let Some(entry) = seat.fallbacks.get_mut(i) {
                if entry.route.provider != route.provider || entry.route.model != route.model {
                    entry.route = route;
                }
            }
        }
        SeatEdit::Option(None, key, value) => {
            seat.route.options.insert(key, value);
        }
        SeatEdit::Option(Some(i), key, value) => {
            if let Some(entry) = seat.fallbacks.get_mut(i) {
                entry.route.options.insert(key, value);
            }
        }
        SeatEdit::AddFallback(route) => seat.fallbacks.push(Fallback {
            route,
            enabled: true,
        }),
        SeatEdit::EnableFallback(i, enabled) => {
            if let Some(entry) = seat.fallbacks.get_mut(i) {
                entry.enabled = enabled;
            }
        }
        SeatEdit::MoveFallback(from, to)
            if from < seat.fallbacks.len() && to < seat.fallbacks.len() =>
        {
            seat.fallbacks.swap(from, to);
        }
        SeatEdit::RemoveFallback(i) if i < seat.fallbacks.len() => {
            seat.fallbacks.remove(i);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apply::edit_docs;
    use skillnet::models::{Capabilities, ModelOption, OptionDescriptor};

    #[test]
    fn groups_routes_by_display_name_not_slug() {
        let models = group_models(
            [
                ("first", "org/model", None, "The Model"),
                ("second", "model", None, "The Model"),
                ("claudeAgent", "claude-opus-5-5", None, "Claude Opus 5.5"),
                (
                    "opencode",
                    "claude-agent/opus",
                    Some("Claude Agent"),
                    "Opus 5.5",
                ),
            ]
            .map(|(id, slug, sub_provider, name)| Provider {
                instance_id: id.into(),
                display_name: id.into(),
                enabled: true,
                status: String::new(),
                models: vec![Model {
                    slug: slug.into(),
                    name: name.into(),
                    sub_provider: sub_provider.map(str::to_owned),
                    capabilities: Default::default(),
                }],
            })
            .into(),
        );
        assert_eq!(models.len(), 2);
        let routes = &models["The Model"];
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].0, "first");
        assert_eq!(routes[0].1.slug, "org/model");
        assert_eq!(routes[1].0, "second");
        assert_eq!(routes[1].1.slug, "model");
        let opus = &models["Claude Opus 5.5"];
        assert_eq!(opus.len(), 2);
        assert_eq!(opus[1].0, "opencode");
        assert_eq!(opus[1].1.slug, "claude-agent/opus");
    }

    #[test]
    fn allows_multiple_opencode_subproviders_as_fallbacks_and_labels_them() {
        let routes = vec![
            (
                "opencode".to_owned(),
                Model {
                    slug: "openai/gpt-6.2-sol".into(),
                    name: "GPT-6.2 Sol".into(),
                    sub_provider: Some("OpenAI".into()),
                    capabilities: Default::default(),
                },
            ),
            (
                "opencode".to_owned(),
                Model {
                    slug: "opencode/gpt-6.2-sol".into(),
                    name: "GPT-6.2 Sol".into(),
                    sub_provider: Some("Personal / OpenCode".into()),
                    capabilities: Default::default(),
                },
            ),
            (
                "opencode".to_owned(),
                Model {
                    slug: "cloudflare-ai-gateway/openai/gpt-6.2-sol".into(),
                    name: "GPT-6.2 Sol".into(),
                    sub_provider: Some("Cloudflare AI Gateway".into()),
                    capabilities: Default::default(),
                },
            ),
        ];
        assert_eq!(
            route_label("opencode", &routes[0].1, &routes),
            "opencode · OpenAI"
        );
        assert_eq!(
            route_label("opencode", &routes[2].1, &routes),
            "opencode · Cloudflare AI Gateway"
        );
        let models = Models::from([("GPT-6.2 Sol".into(), routes)]);
        let mut seat = Seat {
            name: "reviewer".into(),
            label: None,
            route: Route {
                provider: "opencode".into(),
                model: "openai/gpt-6.2-sol".into(),
                options: BTreeMap::new(),
            },
            fallbacks: vec![
                Fallback {
                    route: Route {
                        provider: "opencode".into(),
                        model: "opencode/gpt-6.2-sol".into(),
                        options: BTreeMap::new(),
                    },
                    enabled: true,
                },
                Fallback {
                    route: Route {
                        provider: "opencode".into(),
                        model: "cloudflare-ai-gateway/openai/gpt-6.2-sol".into(),
                        options: BTreeMap::new(),
                    },
                    enabled: true,
                },
            ],
        };
        // Switching primary to another opencode sub-provider drops only that
        // exact route from fallbacks, preserving the other opencode fallback.
        edit_seat(
            &mut seat,
            SeatEdit::Route(
                None,
                Route {
                    provider: "opencode".into(),
                    model: "opencode/gpt-6.2-sol".into(),
                    options: BTreeMap::new(),
                },
            ),
            &models,
        );
        assert_eq!(seat.fallbacks.len(), 1);
        assert_eq!(
            seat.fallbacks[0].route.model,
            "cloudflare-ai-gateway/openai/gpt-6.2-sol"
        );
    }

    #[test]
    fn default_options_falls_back_to_current_value() {
        let model = Model {
            slug: "gemini-4-neon".into(),
            name: "Gemini 4 Neon".into(),
            sub_provider: None,
            capabilities: Capabilities {
                option_descriptors: vec![OptionDescriptor {
                    id: "thought_level".into(),
                    label: "Thinking Level".into(),
                    kind: "select".into(),
                    options: vec![
                        ModelOption {
                            id: "low".into(),
                            label: "Low".into(),
                            is_default: None,
                        },
                        ModelOption {
                            id: "high".into(),
                            label: "High".into(),
                            is_default: None,
                        },
                    ],
                    current_value: Some(serde_json::Value::String("high".into())),
                }],
            },
        };
        let route = new_route("antigravity", &model);
        assert_eq!(
            route.options.get("thought_level").map(String::as_str),
            Some("high")
        );
    }

    #[test]
    fn scopes_edits_without_changing_other_bindings() {
        let binding = SkillBinding {
            seats: vec![Seat {
                name: "reviewer".into(),
                label: None,
                route: Route {
                    provider: "first".into(),
                    model: "org/model".into(),
                    options: BTreeMap::new(),
                },
                fallbacks: vec![],
            }],
        };
        let mut fleet = BindingsDoc::default();
        let mut local = BindingsDoc::default();
        local.skills.insert("other".into(), binding.clone());

        let (new_fleet, new_local) = edit_docs(&fleet, &local, Apply::Local, "skill", &binding);
        assert!(new_fleet.is_none());
        local = new_local.unwrap();
        assert_eq!(local.skills["skill"], binding);

        let (new_fleet, new_local) = edit_docs(&fleet, &local, Apply::Fleet, "skill", &binding);
        fleet = new_fleet.unwrap();
        assert_eq!(fleet.skills["skill"], binding);
        let cleared = new_local.unwrap();
        assert!(!cleared.skills.contains_key("skill"));
        assert_eq!(cleared.skills["other"], binding);

        let (new_fleet, new_local) = edit_docs(&fleet, &local, Apply::Drop, "skill", &binding);
        assert!(new_fleet.is_none());
        assert_eq!(new_local.unwrap(), cleared);
    }
}
