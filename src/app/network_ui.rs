//! Servers in the window: the Network section of the sidebar, the Connect to server, login and
//! question dialogs, and connections that run in the background until their folder can be shown.
use super::*;
use crate::fs::network::{self, Address, Mount, Progress, Prompt, Reply, Server, Servers};
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_secs(4);

/// Where a folder goes once its server is connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenIn {
    Here,
    NewTab,
    NewWindow,
}

pub(super) enum NetMsg {
    Listed(Vec<Mount>),
    Ask { id: u64, prompt: Prompt },
    Done { id: u64, result: Result<PathBuf, Option<String>> },
    Gone { local: PathBuf, name: String, result: Result<(), String> },
    Forgot { name: String, result: Result<(), String> },
}

/// A connection on its way.
struct Pending {
    id: u64,
    address: Address,
    open: OpenIn,
    replies: mpsc::Sender<Reply>,
    /// A login or question dialog waits for the person.
    asking: bool,
    /// Started from the Connect to server dialog, which comes back with the error if it fails.
    from_dialog: bool,
    /// Add the server to the sidebar once connected.
    save: bool,
}

pub(super) struct NetState {
    tx: mpsc::Sender<NetMsg>,
    rx: mpsc::Receiver<NetMsg>,
    pub(super) servers: Servers,
    pub(super) mounts: Vec<Mount>,
    pending: Vec<Pending>,
    next_id: u64,
    listing: bool,
    last: Instant,
    /// Mount folders being disconnected.
    leaving: HashSet<PathBuf>,
}

impl NetState {
    pub(super) fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, servers: Servers::load(), mounts: Vec::new(), pending: Vec::new(), next_id: 1, listing: false, last: Instant::now(), leaving: HashSet::new() }
    }
}

/// A row of the Network section: a saved server, a connected one, or both.
#[derive(Clone)]
pub(super) struct NetRow {
    pub(super) name: String,
    pub(super) icon: &'static str,
    pub(super) address: Option<Address>,
    /// Where a click goes while connected.
    pub(super) open: Option<PathBuf>,
    pub(super) mount: Option<Mount>,
    /// Index into the saved servers.
    pub(super) saved: Option<usize>,
}

/// Where `address` shows inside `mount`.
fn local_in(mount: &Mount, address: &Address) -> PathBuf {
    let base = mount.address.as_ref().map_or("/", |a| a.path.as_str());
    let rest = address.path.strip_prefix(base).unwrap_or("").trim_start_matches('/');
    if rest.is_empty() { mount.local.clone() } else { mount.local.join(rest) }
}

/// What the dialogs and the sidebar asked for, carried out after drawing.
pub(super) enum NetEvent {
    Open(PathBuf),
    Connect(Address, OpenIn),
    NewTab(PathBuf),
    Disconnect(Mount),
    Menu(egui::Pos2, NetRow),
    ConnectDialog,
}

impl SkiApp {
    /// Apply what finished, ask for a new listing every few seconds, and cancel the questions whose dialog was dismissed.
    pub(super) fn poll_network(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.net.rx.try_recv() {
            match msg {
                NetMsg::Listed(mounts) => {
                    self.net.listing = false;
                    self.net.mounts = mounts;
                }
                NetMsg::Ask { id, prompt } => self.show_prompt(id, prompt),
                NetMsg::Done { id, result } => self.connected(ctx, id, result),
                NetMsg::Gone { local, name, result } => {
                    self.net.leaving.remove(&local);
                    match result {
                        Ok(()) => {
                            if self.current_path.starts_with(&local) {
                                self.navigate_to(dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
                            }
                            self.set_toast(trf("Disconnected from “{}”", &[&name]));
                        }
                        Err(err) => self.set_toast(trf("Could not disconnect “{}”: {}", &[&name, &err])),
                    }
                    self.request_network_listing(ctx, true);
                }
                NetMsg::Forgot { name, result } => match result {
                    Ok(()) => self.set_toast(trf("Forgot the password of “{}”", &[&name])),
                    Err(err) => self.set_toast(trf("Could not forget the password: {}", &[&err])),
                },
            }
        }
        // A login or question that was closed some other way (Escape, a click outside) is a cancel
        let shown = match &self.dialog {
            Some(Dialog::Login { id, .. } | Dialog::NetQuestion { id, .. }) => Some(*id),
            _ => None,
        };
        for pending in self.net.pending.iter_mut().filter(|p| p.asking && Some(p.id) != shown) {
            pending.asking = false;
            let _ = pending.replies.send(Reply::Cancel);
        }
        if !self.net.pending.is_empty() || !self.net.leaving.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.request_network_listing(ctx, false);
    }

    pub(super) fn request_network_listing(&mut self, ctx: &egui::Context, now: bool) {
        let state = &mut self.net;
        if state.listing || !now && state.last.elapsed() < REFRESH {
            return;
        }
        state.listing = true;
        state.last = Instant::now();
        let (tx, ctx) = (state.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(NetMsg::Listed(network::list_mounts()));
            ctx.request_repaint();
        });
    }

    /// Show `address`: at once when its server is connected, otherwise once connecting is done.
    pub(crate) fn open_address(&mut self, address: Address, password: Option<String>, open: OpenIn) {
        if let Some(local) = network::local_path(&address).filter(|p| p.exists()) {
            self.show_connected(local, open);
            return;
        }
        self.start_connecting(address, password, open, false, false);
    }

    fn start_connecting(&mut self, address: Address, password: Option<String>, open: OpenIn, from_dialog: bool, save: bool) -> u64 {
        // A second click on a server that is still connecting waits for the first
        if let Some(p) = self.net.pending.iter().find(|p| p.address == address) {
            return p.id;
        }
        let id = self.net.next_id;
        self.net.next_id += 1;
        let (tx, ctx) = (self.net.tx.clone(), self.ctx.clone());
        let replies = network::connect(address.clone(), password, move |progress| {
            let msg = match progress {
                Progress::Ask(prompt) => NetMsg::Ask { id, prompt },
                Progress::Done(result) => NetMsg::Done { id, result },
            };
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
        self.net.pending.push(Pending { id, address, open, replies, asking: false, from_dialog, save });
        id
    }

    /// Whether a connection to the server of `address` is on its way.
    fn connecting(&self, address: &Address) -> bool {
        self.net.pending.iter().any(|p| p.address.same_mount(address))
    }

    fn connected(&mut self, ctx: &egui::Context, id: u64, result: Result<PathBuf, Option<String>>) {
        let Some(at) = self.net.pending.iter().position(|p| p.id == id) else { return };
        let pending = self.net.pending.remove(at);
        let dialog_busy = matches!(&self.dialog, Some(Dialog::Connect { busy: Some(b), .. }) if *b == id);
        match result {
            Ok(local) => {
                // The account that logged in may differ from the one in the address
                let address = network::address_of(&local).unwrap_or_else(|| pending.address.clone());
                self.net.servers.used(&address);
                if pending.save && self.net.servers.find(&address).is_none() {
                    self.net.servers.saved.push(Server { name: String::new(), address });
                }
                self.net.servers.save();
                if dialog_busy {
                    self.dialog = None;
                }
                self.request_network_listing(ctx, true);
                self.show_connected(local, pending.open);
            }
            Err(None) => {
                if let Some(Dialog::Connect { busy, .. }) = &mut self.dialog {
                    *busy = None;
                }
            }
            Err(Some(err)) => {
                if pending.from_dialog {
                    match &mut self.dialog {
                        Some(Dialog::Connect { busy, error, .. }) => {
                            *busy = None;
                            *error = Some(err);
                        }
                        _ => {
                            self.dialog = Some(Dialog::Connect { text: pending.address.display(), save: pending.save, error: Some(err), first_frame: true, busy: None });
                        }
                    }
                } else {
                    self.set_toast(trf("Could not connect to “{}”: {}", &[&pending.address.name(), &err]));
                }
            }
        }
    }

    /// Show a folder of a connected server; a file is shown selected in its folder.
    fn show_connected(&mut self, local: PathBuf, open: OpenIn) {
        let (folder, select) = if local.is_dir() || !local.exists() {
            (local, None)
        } else {
            (local.parent().map(Path::to_path_buf).unwrap_or_else(|| local.clone()), Some(local))
        };
        match open {
            OpenIn::Here => {
                self.close_search();
                self.navigate_to(folder);
                if let Some(file) = select {
                    self.select_path(&file);
                }
            }
            OpenIn::NewTab => self.new_tab(folder),
            OpenIn::NewWindow => self.open_window(&folder),
        }
    }

    /// The login or question dialog for a connection that waits for an answer.
    fn show_prompt(&mut self, id: u64, prompt: Prompt) {
        let Some(pending) = self.net.pending.iter_mut().find(|p| p.id == id) else { return };
        pending.asking = true;
        let address = pending.address.clone();
        self.dialog = Some(match prompt {
            Prompt::Login { user, domain, fixed_user: _, anonymous, retry } => Dialog::Login {
                id,
                server: address.name(),
                user: user.or_else(|| address.user.clone()).unwrap_or_else(|| if address.scheme == network::Scheme::Sftp { std::env::var("USER").unwrap_or_default() } else { String::new() }),
                domain: domain.or_else(|| address.domain.clone()).unwrap_or_default(),
                password: String::new(),
                show_domain: address.scheme.has_domain(),
                anonymous: anonymous.then(|| address.scheme.anonymous_label()),
                retry,
                remember: true,
                reveal: false,
                first_frame: true,
            },
            Prompt::Question { message, choices } => {
                let (title, body) = describe_question(&message);
                Dialog::NetQuestion { id, title, body, choices: choices.iter().map(|c| choice_label(c)).collect() }
            }
        });
    }

    fn reply(&mut self, id: u64, reply: Reply) {
        if let Some(pending) = self.net.pending.iter_mut().find(|p| p.id == id) {
            pending.asking = false;
            let _ = pending.replies.send(reply);
        }
    }

    pub(super) fn disconnect(&mut self, ctx: &egui::Context, mount: Mount) {
        if !self.net.leaving.insert(mount.local.clone()) {
            return;
        }
        // Nothing may hold the folder open: leave it first
        if self.current_path.starts_with(&mount.local) {
            self.navigate_to(dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        }
        let (tx, ctx) = (self.net.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = network::disconnect(&mount);
            let _ = tx.send(NetMsg::Gone { local: mount.local.clone(), name: mount.name.clone(), result });
            ctx.request_repaint();
        });
    }

    pub(super) fn forget_password(&mut self, address: Address) {
        let (tx, ctx) = (self.net.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let result = network::forget_password(&address);
            let _ = tx.send(NetMsg::Forgot { name: address.name(), result });
            ctx.request_repaint();
        });
    }

    pub(super) fn save_server(&mut self, address: Address) {
        if self.net.servers.find(&address).is_none() {
            self.net.servers.saved.push(Server { name: String::new(), address });
            self.net.servers.save();
        }
    }

    pub(super) fn remove_server(&mut self, index: usize) {
        if index < self.net.servers.saved.len() {
            self.net.servers.saved.remove(index);
            self.net.servers.save();
        }
    }

    pub(crate) fn open_connect_dialog(&mut self) {
        let text = network::display_path(&self.current_path).filter(|_| self.chooser.is_none()).unwrap_or_default();
        self.dialog = Some(Dialog::Connect { text, save: true, error: None, first_frame: true, busy: None });
    }

    pub(super) fn open_edit_server(&mut self, index: usize) {
        let Some(server) = self.net.servers.saved.get(index) else { return };
        self.dialog = Some(Dialog::EditServer { index, name: server.label(), text: server.address.display(), error: None, first_frame: true });
    }

    /// The server's own name for the sidebar or the trail of folders: a saved name, else the address's.
    pub(super) fn server_label(&self, mount: &Mount) -> String {
        mount.address.as_ref().and_then(|a| self.net.servers.find(a)).map(|i| self.net.servers.saved[i].label()).unwrap_or_else(|| mount.name.clone())
    }

    /// The Network section: saved servers, connected ones, and the way to add one.
    pub(super) fn network_rows(&self) -> Vec<NetRow> {
        let mounts = &self.net.mounts;
        let mut used = vec![false; mounts.len()];
        let mut rows = Vec::new();
        for (index, server) in self.net.servers.saved.iter().enumerate() {
            let found = mounts.iter().position(|m| m.address.as_ref().is_some_and(|a| a.same_mount(&server.address)));
            if let Some(i) = found {
                used[i] = true;
            }
            let mount = found.map(|i| mounts[i].clone());
            rows.push(NetRow {
                name: server.label(),
                icon: icons::SERVER_NETWORK,
                address: Some(server.address.clone()),
                open: mount.as_ref().map(|m| local_in(m, &server.address)),
                mount,
                saved: Some(index),
            });
        }
        for (mount, _) in mounts.iter().zip(used).filter(|(_, used)| !used) {
            rows.push(NetRow {
                name: mount.name.clone(),
                icon: if mount.is_device() { icons::CELLPHONE } else { icons::SERVER_NETWORK },
                address: mount.address.clone(),
                open: Some(mount.local.clone()),
                mount: Some(mount.clone()),
                saved: None,
            });
        }
        rows
    }

    /// Draw the Network section of the sidebar; what was clicked comes back as events.
    pub(super) fn network_sidebar(&self, ui: &mut egui::Ui, browsing: bool, zones: &mut Vec<dnd::DropZone>) -> Vec<NetEvent> {
        let mut events = Vec::new();
        let theme = self.theme;
        for (idx, row) in self.network_rows().into_iter().enumerate() {
            let busy = row.address.as_ref().is_some_and(|a| self.connecting(a)) || row.mount.as_ref().is_some_and(|m| self.net.leaving.contains(&m.local));
            let reserve = if row.mount.is_some() || busy { material::NAV_TRAILING } else { 0.0 };
            let here = browsing && row.open.as_ref().is_some_and(|p| *p == self.current_path);
            let resp = material::nav_item_with(ui, ("net", idx, &row.name), row.icon, &row.name, &theme, here, reserve);
            if busy {
                let spot = egui::Rect::from_center_size(egui::pos2(resp.rect.right() - 8.0 - 14.0, resp.rect.center().y), egui::vec2(16.0, 16.0));
                ui.put(spot, egui::Spinner::new().size(14.0).color(theme.subtext0));
            } else if let Some(mount) = &row.mount
                && drives_ui::row_button(ui, resp.rect, resp.hovered(), ("net_off", idx), icons::UNMOUNT, tr("Disconnect"), &theme)
            {
                events.push(NetEvent::Disconnect(mount.clone()));
                continue;
            }
            if let Some(open) = &row.open {
                zones.push(dnd::DropZone { rect: resp.rect, dest: open.clone() });
            }
            if resp.clicked() {
                match (&row.open, &row.address) {
                    (Some(open), _) => events.push(NetEvent::Open(open.clone())),
                    (None, Some(address)) if !busy => events.push(NetEvent::Connect(address.clone(), OpenIn::Here)),
                    _ => {}
                }
            }
            if resp.middle_clicked() {
                match (&row.open, &row.address) {
                    (Some(open), _) => events.push(NetEvent::NewTab(open.clone())),
                    (None, Some(address)) => events.push(NetEvent::Connect(address.clone(), OpenIn::NewTab)),
                    _ => {}
                }
            }
            if resp.secondary_clicked()
                && let Some(pos) = resp.interact_pointer_pos()
            {
                events.push(NetEvent::Menu(pos, row.clone()));
            }
        }
        if material::nav_item(ui, "net_connect", icons::LAN_CONNECT, tr("Connect to server…"), &theme, false).clicked() {
            events.push(NetEvent::ConnectDialog);
        }
        events
    }

    pub(super) fn apply_network_events(&mut self, ctx: &egui::Context, events: Vec<NetEvent>) {
        for event in events {
            match event {
                NetEvent::Open(path) => {
                    self.close_search();
                    self.navigate_to(path);
                }
                NetEvent::Connect(address, open) => self.open_address(address, None, open),
                NetEvent::NewTab(path) => self.new_tab(path),
                NetEvent::Disconnect(mount) => self.disconnect(ctx, mount),
                NetEvent::Menu(pos, row) => self.open_menu(ctx, pos, MenuTarget::Server(Box::new(row))),
                NetEvent::ConnectDialog => self.open_connect_dialog(),
            }
        }
    }

    /// Context menu of a Network row.
    pub(super) fn server_menu(&self, row: &NetRow) -> Vec<Vec<MenuEntry>> {
        let mut groups = Vec::new();
        match (&row.open, &row.address) {
            (Some(open), _) => groups.push(vec![
                (icons::OPEN, tr("Open"), MenuAction::OpenPath(open.clone())),
                (icons::NEW_TAB, tr("Open in new tab"), MenuAction::OpenInNewTab(open.clone())),
                (icons::NEW_WINDOW, tr("Open in new window"), MenuAction::OpenInNewWindow(open.clone())),
            ]),
            (None, Some(address)) => groups.push(vec![
                (icons::LAN_CONNECT, tr("Connect"), MenuAction::Connect(address.clone(), OpenIn::Here)),
                (icons::NEW_TAB, tr("Open in new tab"), MenuAction::Connect(address.clone(), OpenIn::NewTab)),
                (icons::NEW_WINDOW, tr("Open in new window"), MenuAction::Connect(address.clone(), OpenIn::NewWindow)),
            ]),
            (None, None) => {}
        }
        if let Some(mount) = &row.mount {
            groups.push(vec![(icons::UNMOUNT, tr("Disconnect"), MenuAction::Disconnect(mount.local.clone()))]);
        }
        match (row.saved, &row.address) {
            (Some(index), _) => groups.push(vec![
                (icons::RENAME, tr("Edit…"), MenuAction::EditServer(index)),
                (icons::PIN_OFF, tr("Remove from sidebar"), MenuAction::RemoveServer(index)),
            ]),
            (None, Some(address)) => groups.push(vec![(icons::PIN, tr("Keep in sidebar"), MenuAction::SaveServer(address.clone()))]),
            (None, None) => {}
        }
        let mut last = Vec::new();
        if let Some(address) = &row.address {
            last.push((icons::COPY, tr("Copy address"), MenuAction::CopyPath(PathBuf::from(address.display()))));
            last.push((icons::KEY, tr("Forget password"), MenuAction::ForgetPassword(address.clone())));
        }
        if let Some(open) = &row.open {
            last.push((icons::INFO, tr("Properties"), MenuAction::Properties(open.clone())));
        }
        groups.push(last);
        groups.retain(|g| !g.is_empty());
        groups
    }

    // --- Dialogs --------------------------------------------------------------------------------

    pub(super) fn connect_dialog_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Connect { text, save, error, first_frame, busy } = dialog else { return false };
        let theme = self.theme;
        let mut close = material::dialog_header(ui, tr("Connect to server"), Some(tr("Windows shares, SSH, FTP and WebDAV")), &theme, "conn_close");

        let mut submit = false;
        material::input_frame(&theme).show(ui, |ui| {
            let out = egui::TextEdit::singleline(text)
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .hint_text("smb://server/share")
                .font(egui::FontId::proportional(material::FONT_SUBTITLE))
                .interactive(busy.is_none())
                .show(ui);
            if std::mem::take(first_frame) {
                out.response.request_focus();
            }
            if out.response.changed() {
                *error = None;
            }
            submit = out.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        });

        // The kinds of server as chips: a click puts its scheme at the start of the address
        let kinds = [network::Scheme::Smb, network::Scheme::Sftp, network::Scheme::Ftp, network::Scheme::Davs];
        let current = Address::parse(text).map(|a| a.scheme);
        ui.horizontal_wrapped(|ui| {
            for scheme in kinds {
                let label = match scheme {
                    network::Scheme::Smb => "SMB",
                    network::Scheme::Sftp => "SFTP",
                    network::Scheme::Ftp => "FTP",
                    _ => "WebDAV",
                };
                if chip(ui, ("conn_kind", scheme.name()), None, label, &theme, current == Some(scheme)).clicked() && busy.is_none() {
                    let rest = match text.split_once("://") {
                        Some((_, rest)) => rest.to_owned(),
                        None => text.trim_start_matches(['\\', '/']).replace('\\', "/"),
                    };
                    *text = format!("{}://{rest}", scheme.name());
                    *error = None;
                    *first_frame = true;
                }
            }
        });

        let parsed = Address::parse_login(text);
        let note = match &parsed {
            Some((address, _)) => format!("{} · {}", address.scheme.label(), address.name()),
            None if text.trim().is_empty() => tr("Type an address such as sftp://me@example.com or smb://nas/media").to_owned(),
            None => tr("Start the address with smb://, sftp://, ftp:// or davs://").to_owned(),
        };
        ui.label(egui::RichText::new(note).size(material::FONT_CAPTION).color(theme.subtext1));
        if let Some(err) = error.as_ref() {
            ui.label(egui::RichText::new(err).size(material::FONT_CAPTION).color(theme.red));
        }

        if material::toggle_row(ui, "conn_save", tr("Keep in sidebar"), None, &theme, *save).clicked() {
            *save = !*save;
        }

        if !self.net.servers.recent.is_empty() {
            material::section_label(ui, tr("Recent servers"), &theme);
            let mut picked = None;
            material::group_card(ui, &theme, |ui| {
                egui::ScrollArea::vertical().max_height(material::ENTRY_HEIGHT * 4.0 + 8.0).auto_shrink([false, true]).id_salt("conn_recent").show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (i, address) in self.net.servers.recent.iter().enumerate() {
                        let resp = nav_item(ui, ("conn_recent", i), icons::SERVER_NETWORK, &address.display(), &theme, false);
                        if resp.double_clicked() {
                            picked = Some((address.display(), true));
                        } else if resp.clicked() {
                            picked = Some((address.display(), false));
                        }
                    }
                });
            });
            if let Some((address, go)) = picked.filter(|_| busy.is_none()) {
                *text = address;
                *error = None;
                submit |= go;
            }
        }

        let label = if busy.is_some() { tr("Connecting…") } else { tr("Connect") };
        let footer = material::dialog_footer(ui, &theme, FooterSpec { enabled: parsed.is_some() && busy.is_none(), ..FooterSpec::new("conn", icons::LAN_CONNECT, label) });
        close |= footer.cancel;
        if (footer.primary || submit) && busy.is_none() {
            match parsed {
                Some((address, password)) => {
                    *error = None;
                    if let Some(local) = network::local_path(&address).filter(|p| p.exists()) {
                        if *save {
                            self.save_server(address);
                        }
                        self.show_connected(local, OpenIn::Here);
                        return true;
                    }
                    *busy = Some(self.start_connecting(address, password, OpenIn::Here, true, *save));
                }
                None => *error = Some(tr("This is not a server address").to_owned()),
            }
        }
        if busy.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        close
    }

    pub(super) fn login_dialog_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::Login { id, server, user, domain, password, show_domain, anonymous, retry, remember, reveal, first_frame } = dialog else { return false };
        let theme = self.theme;
        let id = *id;
        let mut close = material::dialog_header(ui, tr("Log in"), Some(server.as_str()), &theme, "login_close");
        if *retry {
            ui.label(egui::RichText::new(tr("The server did not accept this user name or password. Try again.")).size(material::FONT_CAPTION).color(theme.red));
        }

        let focus = std::mem::take(first_frame);
        let mut submit = false;
        let field = |ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str, secret: bool, focus: bool| -> (bool, egui::Rect) {
            ui.label(egui::RichText::new(label).size(material::FONT_CAPTION).color(theme.subtext1));
            let mut entered = false;
            let frame = material::input_frame(&theme).show(ui, |ui| {
                let out = egui::TextEdit::singleline(value)
                    .frame(egui::Frame::NONE)
                    .desired_width(if secret { ui.available_width() - 28.0 } else { f32::INFINITY })
                    .hint_text(hint)
                    .password(secret)
                    .font(egui::FontId::proportional(material::FONT_SUBTITLE))
                    .show(ui);
                if focus {
                    out.response.request_focus();
                }
                entered = out.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            });
            (entered, frame.response.rect)
        };
        submit |= field(ui, tr("User name"), user, "", false, focus && user.is_empty()).0;
        if *show_domain {
            submit |= field(ui, tr("Domain"), domain, "WORKGROUP", false, false).0;
        }
        let (entered, rect) = field(ui, tr("Password"), password, "", !*reveal, focus && !user.is_empty());
        submit |= entered;
        // The eye at the end of the password field shows what was typed
        let eye = egui::Rect::from_min_size(egui::pos2(rect.right() - 6.0 - 28.0, rect.center().y - 14.0), egui::vec2(28.0, 28.0));
        let (icon, tip) = if *reveal { (icons::VISIBILITY_OFF, tr("Hide password")) } else { (icons::VISIBILITY, tr("Show password")) };
        if ui.scope_builder(egui::UiBuilder::new().max_rect(eye), |ui| icon_button(ui, "login_reveal", icon, &theme, true, tip)).inner.clicked() {
            *reveal = !*reveal;
        }

        ui.add_space(2.0);
        if material::toggle_row(ui, "login_remember", tr("Remember password"), None, &theme, *remember).clicked() {
            *remember = !*remember;
        }

        let footer = material::dialog_footer(ui, &theme, FooterSpec { extra: *anonymous, ..FooterSpec::new("login", icons::LOCK_OPEN, tr("Connect")) });
        if footer.primary || submit {
            let reply = Reply::Login { user: user.trim().to_owned(), domain: domain.trim().to_owned(), password: std::mem::take(password), remember: *remember };
            self.reply(id, reply);
            return true;
        }
        if footer.extra {
            self.reply(id, Reply::Anonymous);
            return true;
        }
        close |= footer.cancel;
        if close {
            self.reply(id, Reply::Cancel);
        }
        close
    }

    pub(super) fn net_question_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::NetQuestion { id, title, body, choices } = dialog else { return false };
        let theme = self.theme;
        let id = *id;
        let close = material::dialog_header(ui, title, None, &theme, "netq_close");
        ui.label(egui::RichText::new(body.as_str()).size(material::FONT_BODY).color(theme.subtext0));
        ui.add_space(8.0);
        let mut picked = None;
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 36.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The first choice goes ahead; it stands at the right, as the main button of every dialog does
            for (i, choice) in choices.iter().enumerate() {
                let kind = if i == 0 { ButtonKind::Filled } else { ButtonKind::Text };
                let resp = material::button(ui, ("netq", i), None, choice, &theme, kind, true);
                if material::dialog_fresh(ui.ctx()) && i + 1 == choices.len() {
                    // Like a destructive action, an unknown server starts on the cautious answer
                    resp.request_focus();
                }
                if resp.clicked() {
                    picked = Some(i);
                }
            }
        });
        if let Some(i) = picked {
            self.reply(id, Reply::Choice(i));
            return true;
        }
        if close {
            self.reply(id, Reply::Cancel);
        }
        close
    }

    pub(super) fn edit_server_contents(&mut self, ui: &mut egui::Ui, dialog: &mut Dialog) -> bool {
        let Dialog::EditServer { index, name, text, error, first_frame } = dialog else { return false };
        let theme = self.theme;
        let mut close = material::dialog_header(ui, tr("Edit server"), None, &theme, "edsrv_close");
        let mut submit = false;
        let focus = std::mem::take(first_frame);
        for (label, value, hint, first) in [(tr("Name"), &mut *name, "", true), (tr("Address"), &mut *text, "smb://server/share", false)] {
            ui.label(egui::RichText::new(label).size(material::FONT_CAPTION).color(theme.subtext1));
            material::input_frame(&theme).show(ui, |ui| {
                let out = egui::TextEdit::singleline(value)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .hint_text(hint)
                    .font(egui::FontId::proportional(material::FONT_SUBTITLE))
                    .show(ui);
                if focus && first {
                    out.response.request_focus();
                }
                if out.response.changed() {
                    *error = None;
                }
                submit |= out.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            });
        }
        if let Some(err) = error.as_ref() {
            ui.label(egui::RichText::new(err).size(material::FONT_CAPTION).color(theme.red));
        }
        let footer = material::dialog_footer(ui, &theme, FooterSpec::new("edsrv", icons::CHECK, tr("Save")));
        close |= footer.cancel;
        if footer.primary || submit {
            match Address::parse(text) {
                Some(address) if *index < self.net.servers.saved.len() => {
                    let label = name.trim();
                    // A name left as the address would show keeps following the address
                    let name = if label == address.name() { String::new() } else { label.to_owned() };
                    self.net.servers.saved[*index] = Server { name, address };
                    self.net.servers.save();
                    close = true;
                }
                Some(_) => close = true,
                None => *error = Some(tr("This is not a server address").to_owned()),
            }
        }
        close
    }
}

/// Title and text for a question a server asks. GVfs asks in English; the ones known here are
/// told in the interface's language.
fn describe_question(message: &str) -> (String, String) {
    if message.contains("identity of the remote computer") {
        let host = message.split_once('(').and_then(|(_, r)| r.split_once(')')).map(|(h, _)| h.to_owned()).unwrap_or_default();
        let print = message.split("remote computer is ").nth(1).and_then(|r| r.split_whitespace().next()).map(|p| p.trim_end_matches('.').to_owned()).unwrap_or_default();
        let body = format!(
            "{}\n\n{}",
            trf("Ski has not connected to {} before, so it can't be sure the server is the one it claims to be. Its key fingerprint is {}.", &[&host, &print]),
            tr("Connect only if you trust this network, or if the fingerprint matches the one the server's administrator gave you."),
        );
        return (tr("Unknown server").to_owned(), body);
    }
    (tr("The server asks").to_owned(), message.to_owned())
}

/// A choice GVfs offers, in the interface's language when it is a known one.
fn choice_label(choice: &str) -> String {
    match choice {
        "Log In Anyway" => tr("Connect anyway").to_owned(),
        "Cancel Login" | "Cancel" => tr("Cancel").to_owned(),
        "Yes" => tr("Yes").to_owned(),
        "No" => tr("No").to_owned(),
        other => other.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_key_questions_are_retold() {
        let message = "The identity of the remote computer (example.com) is unknown.\nThe identity sent by the remote computer is SHA256:abc123.";
        let (title, body) = describe_question(message);
        assert_eq!(title, "Unknown server");
        assert!(body.contains("example.com") && body.contains("SHA256:abc123") && !body.contains("abc123.."));
        assert_eq!(choice_label("Log In Anyway"), "Connect anyway");
        assert_eq!(choice_label("Something"), "Something");
    }

    #[test]
    fn saved_servers_find_their_folder_inside_a_mount() {
        let mount = Mount {
            name: "x".into(),
            address: Address::parse("smb://nas/media"),
            local: PathBuf::from("/run/user/1/gvfs/smb-share:server=nas,share=media"),
            kind: network::MountKind::Gvfs,
        };
        let deep = Address::parse("smb://nas/media/films/2024").unwrap();
        assert_eq!(local_in(&mount, &deep), mount.local.join("films/2024"));
        assert_eq!(local_in(&mount, &Address::parse("smb://nas/media").unwrap()), mount.local);
    }
}
