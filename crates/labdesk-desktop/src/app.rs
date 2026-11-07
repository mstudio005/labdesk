// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Connects the LabDesk screens to `labdesk-core`. No lab rules live here: this file reads what the
//! person typed, asks the core to do the work, and shows the answer (numbers, inline field errors, toasts).
use crate::{diag, fmt, AnBar, AnKpi, AnRow, AppWindow, SmRow, TrPoint, CountRow, DeletedRow, HelpRow, ARow, FRow, OrderRow, ParamRow, PatRow, PayRow, SumRow, MachRow, MachItemRow, HistRow, CandRow, TestAdminRow, TestRow, TreeRow, UserRow};
use crate::updater;
use labdesk_core::calc::{calculate_results, flag_for, result_range};
use labdesk_core::{Actor, BackupSettings, Error, NewOrder, NewPatient, Order, OrderItem, Parameter, Profile, Referral, ResultInput, ResultRow, Role, Store, TestConfig, TestDef, User};
use slint::{ComponentHandle, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode, VecModel, Weak};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

/// Eight made-up tests so a new lab can try the app. Not a clinical reference.
pub const DEMO_CATALOGUE: &str = include_str!("../demo-catalogue.json");
/// The Terms of Use and Privacy text, the same file the installer and the website show.
pub const TERMS_TEXT: &str = include_str!("../../../TERMS.txt");
/// The lab's standard list of 283 tests (from the web version), with reference ranges. Prices are not set.
pub const STANDARD_TESTS: &str = include_str!("../assets/standard-tests.json");

const FILTERS: [&str; 5] = ["ALL", "WAITING", "PARTIAL", "READY", "DELIVERED"];
const STATUS: [&str; 5] = ["", "Pending", "Partial", "Completed", "Delivered"];
/// The order of the role buttons on the People screen.
const ROLE_ORDER: [Role; 4] = [Role::Reception, Role::Technician, Role::Pathologist, Role::Admin];

/// Where LabDesk keeps things outside the database.
#[derive(Clone)]
pub struct Options {
    /// Reports, test pages and backups are saved under `<docs_dir>/Reports` and `<docs_dir>/Backups`.
    pub docs_dir: PathBuf,
    /// Open saved PDFs in the computer's PDF viewer (off in automated tests).
    pub open_files: bool,
    /// The folder that holds the database. The saved sign-in and crash notes live here too.
    pub data_dir: PathBuf,
    /// The database file (shown on the About page).
    pub db_path: PathBuf,
    /// Remember who is signed in between runs (off in automated tests).
    pub use_sessions: bool,
}
impl Options {
    pub fn new() -> Self {
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        Self { docs_dir: home.join("Documents").join("LabDesk"), open_files: true, data_dir: PathBuf::from("."), db_path: PathBuf::new(), use_sessions: false }
    }
}
impl Default for Options { fn default() -> Self { Self::new() } }

/// What the mail thread reports back: the result, the visit (0 for a test), and the address.
struct MailDone { result: Result<(), String>, order: i64, to: String }
/// An action waiting for the person to answer the "Are you sure?" box.
#[derive(Clone, Copy, PartialEq)]
enum Pending { SendEmail(i64), SamePerson(i64), SaveUser, CreateUser, RemoveUser(i64), NewRecovery, DeleteOrder(i64), Restore, RestoreFirstRun }

struct State {
    store: Store,
    user: Option<User>,
    pending: Option<Pending>,
    /// The backup file the person picked, while the "Are you sure?" box is open.
    restore_file: Option<PathBuf>,
    doctors: Rc<VecModel<SharedString>>,
    help: Rc<VecModel<HelpRow>>,
    deleted: Rc<VecModel<DeletedRow>>,
    /// The test being edited on the Tests screen, and its result lines exactly as typed.
    te_base: Option<TestDef>,
    tp: Vec<Parameter>,
    sel: Vec<bool>,
    te_cfg: TestConfig,
    fp_ranges: Option<Vec<labdesk_core::RangeOverlay>>,
    ar: Vec<ARow>,
    ci_kind: i32,
    interp_plain: String,
    copy_ids: Vec<i64>,
    copy_model: Rc<VecModel<TestAdminRow>>,
    ar_model: Rc<VecModel<ARow>>,
    admin_tests: Rc<VecModel<TestAdminRow>>,
    row_model: Rc<VecModel<FRow>>,
    users: Rc<VecModel<UserRow>>,
    selected: BTreeSet<i64>,
    order: Option<Order>,
    /// What is currently typed on the result sheet: (order_test_id, parameter_id) -> text.
    entries: HashMap<(i64, String), String>,
    orders: Rc<VecModel<OrderRow>>,
    attention: Rc<VecModel<OrderRow>>,
    tests: Rc<VecModel<TestRow>>,
    params: Rc<VecModel<ParamRow>>,
    filters: Rc<VecModel<SharedString>>,
    pats: Rc<VecModel<PatRow>>,
    finds: Rc<VecModel<PatRow>>,
    pat_tests: Rc<VecModel<CountRow>>,
    pat_tree: Rc<VecModel<TreeRow>>,
    /// The returning patient the registration form is filled from, if any.
    linked: Option<i64>,
    /// Hints for "already registered?" shown while typing a new patient.
    matches: Rc<VecModel<PatRow>>,
    /// The person said "no, this is a new person", so do not ask again for this form.
    skip_match: bool,
    pay_target: Option<PayTarget>,
    pat_pays: Rc<VecModel<PayRow>>,
    ap_rows: Rc<VecModel<SumRow>>,
    sm_rows: Rc<VecModel<SmRow>>,
    trends: Vec<labdesk_core::Trend>,
    tr_points: Rc<VecModel<TrPoint>>,
    tr_names: Rc<VecModel<SharedString>>,
    sc_rows: Rc<VecModel<SumRow>>,
    mach_list: Rc<VecModel<MachRow>>,
    md_items: Rc<VecModel<MachItemRow>>,
    md_hists: Rc<VecModel<HistRow>>,
    md_cands: Rc<VecModel<CandRow>>,
}
#[derive(Clone, Copy)]
enum PayTarget { Order(i64), Patient(i64) }

pub struct Ctl { mail_rx: RefCell<Option<std::sync::mpsc::Receiver<MailDone>>>, mail_timer: Timer, win_drag: std::cell::Cell<(f32, f32)>, win_normal: std::cell::Cell<(f32, f32)>, upd_again: Timer, win_timer: Timer, crash_text: RefCell<String>, upd_rx: RefCell<Option<std::sync::mpsc::Receiver<updater::Outcome>>>, upd_timer: Timer, print_after: std::cell::Cell<bool>, rx: RefCell<Option<std::sync::mpsc::Receiver<String>>>, rx_timer: Timer, capture: RefCell<Option<labdesk_core::capture::Capture>>, ui: Weak<AppWindow>, st: RefCell<State>, opts: Options, toast_timer: Timer, lock_timer: Timer, backup_timer: Timer, me: RefCell<std::rc::Weak<Ctl>> }

/// Connect `ui` to `store` and show the home screen. Keep the returned handle alive while the UI runs.
pub fn wire(ui: &AppWindow, store: Store, opts: Options) -> Rc<Ctl> {
    let st = State { store, user: None, pending: None, restore_file: None, doctors: Rc::new(VecModel::default()), help: Rc::new(VecModel::default()), deleted: Rc::new(VecModel::default()), te_base: None, tp: vec![], sel: vec![], te_cfg: TestConfig::default(), fp_ranges: None, ar: vec![], ci_kind: 0, interp_plain: String::new(), copy_ids: vec![], copy_model: Rc::new(VecModel::default()), ar_model: Rc::new(VecModel::default()), admin_tests: Rc::new(VecModel::default()), row_model: Rc::new(VecModel::default()), users: Rc::new(VecModel::default()), selected: BTreeSet::new(), order: None, entries: HashMap::new(),
        orders: Rc::new(VecModel::default()), attention: Rc::new(VecModel::default()), tests: Rc::new(VecModel::default()),
        params: Rc::new(VecModel::default()), filters: Rc::new(VecModel::default()),
        pats: Rc::new(VecModel::default()), finds: Rc::new(VecModel::default()), pat_tests: Rc::new(VecModel::default()), pat_tree: Rc::new(VecModel::default()), linked: None, matches: Rc::new(VecModel::default()), skip_match: false, pay_target: None, pat_pays: Rc::new(VecModel::default()), ap_rows: Rc::new(VecModel::default()), sm_rows: Rc::new(VecModel::default()), trends: vec![], tr_points: Rc::new(VecModel::default()), tr_names: Rc::new(VecModel::default()), sc_rows: Rc::new(VecModel::default()), mach_list: Rc::new(VecModel::default()), md_items: Rc::new(VecModel::default()), md_hists: Rc::new(VecModel::default()), md_cands: Rc::new(VecModel::default()) };
    ui.set_orders(ModelRc::from(st.orders.clone()));
    ui.set_attention(ModelRc::from(st.attention.clone()));
    ui.set_tests(ModelRc::from(st.tests.clone()));
    ui.set_params(ModelRc::from(st.params.clone()));
    ui.set_filter_labels(ModelRc::from(st.filters.clone()));
    ui.set_tests_admin(ModelRc::from(st.admin_tests.clone()));
    ui.set_frows(ModelRc::from(st.row_model.clone()));
    ui.set_copy_hits(ModelRc::from(st.copy_model.clone()));
    ui.set_ar_rows(ModelRc::from(st.ar_model.clone()));
    ui.set_users(ModelRc::from(st.users.clone()));
    ui.set_auth_first_run(!st.store.has_users().unwrap_or(true));
    ui.set_terms_text(TERMS_TEXT.into());
    ui.set_terms_needed(!st.store.terms_accepted().unwrap_or(false));
    ui.set_doctor_suggestions(ModelRc::from(st.doctors.clone()));
    ui.set_help_items(ModelRc::from(st.help.clone()));
    ui.set_deleted(ModelRc::from(st.deleted.clone()));
    ui.set_pat_list(ModelRc::from(st.pats.clone()));
    ui.set_find_results(ModelRc::from(st.finds.clone()));
    ui.set_match_results(ModelRc::from(st.matches.clone()));
    ui.set_pat_tests(ModelRc::from(st.pat_tests.clone()));
    ui.set_pat_tree(ModelRc::from(st.pat_tree.clone()));
    ui.set_pat_pays(ModelRc::from(st.pat_pays.clone()));
    ui.set_ap_rows(ModelRc::from(st.ap_rows.clone()));
    ui.set_sm_rows(ModelRc::from(st.sm_rows.clone()));
    ui.set_tr_points(ModelRc::from(st.tr_points.clone()));
    ui.set_tr_names(ModelRc::from(st.tr_names.clone()));
    ui.set_sc_rows(ModelRc::from(st.sc_rows.clone()));
    ui.set_mach_list(ModelRc::from(st.mach_list.clone()));
    ui.set_md_items(ModelRc::from(st.md_items.clone()));
    ui.set_md_hists(ModelRc::from(st.md_hists.clone()));
    ui.set_md_cands(ModelRc::from(st.md_cands.clone()));
    ui.set_app_version(env!("CARGO_PKG_VERSION").into());
    ui.set_about_path(opts.db_path.display().to_string().into());
    let c = Rc::new(Ctl { mail_rx: RefCell::new(None), mail_timer: Timer::default(), win_drag: std::cell::Cell::new((0.0, 0.0)), win_normal: std::cell::Cell::new((1360.0, 820.0)), upd_again: Timer::default(), win_timer: Timer::default(), crash_text: RefCell::new(String::new()), upd_rx: RefCell::new(None), upd_timer: Timer::default(), print_after: std::cell::Cell::new(false), rx: RefCell::new(None), rx_timer: Timer::default(), capture: RefCell::new(None), ui: ui.as_weak(), st: RefCell::new(st), opts, toast_timer: Timer::default(), lock_timer: Timer::default(), backup_timer: Timer::default(), me: RefCell::new(std::rc::Weak::new()) });
    *c.me.borrow_mut() = Rc::downgrade(&c);

    macro_rules! on { ($setter:ident, |$c:ident $(, $a:ident)*| $body:expr) => {{ let $c = c.clone(); ui.$setter(move |$($a),*| $body); }}; }
    on!(on_navigate, |c, i| c.navigate(i));
    on!(on_pat_query_edited, |c| c.refresh_patients());
    on!(on_pick_pat, |c, id| c.show_patient(id as i64));
    on!(on_new_visit, |c| c.new_visit());
    on!(on_find_edited, |c, q| c.find_edited(q.as_str()));
    on!(on_pick_patient, |c, id| c.pick_patient(id as i64));
    on!(on_unlink_patient, |c| c.unlink_patient());
    on!(on_refresh_orders, |c| c.refresh_orders());
    on!(on_open_order, |c, id| c.open_order(id as i64));
    on!(on_toggle_test, |c, id| c.toggle_test(id as i64));
    on!(on_test_query_edited, |c, _q| c.refresh_tests());
    on!(on_bill_changed, |c| c.refresh_bill());
    on!(on_register, |c| c.register());
    on!(on_value_edited, |c, i, t| c.value_edited(i as usize, t.as_str()));
    on!(on_save_draft, |c| { c.save_draft(true); });
    on!(on_approve, |c| c.approve());
    on!(on_ask_correction, |c| c.set_asking(true));
    on!(on_cancel_correction, |c| c.set_asking(false));
    on!(on_confirm_correction, |c| c.confirm_correction());
    on!(on_deliver, |c| c.deliver());
    on!(on_make_report, |c| c.make_report());
    on!(on_tr_choose, |c, n| c.trend_show(n.as_str()));
    on!(on_win_minimize, |c| c.ui().window().set_minimized(true));
    on!(on_win_maximize, |c| { let w = c.ui(); let m = w.window().is_maximized(); w.window().set_maximized(!m); w.set_win_maxed(!m); });
    on!(on_win_close, |c| { c.before_exit(); let _ = c.ui().hide(); let _ = slint::quit_event_loop(); });
    on!(on_win_drag_down, |c, x, y| c.win_drag.set((x, y)));
    on!(on_win_drag_to, |c, x, y| c.window_drag(x, y));
    {
        // closing the window any other way (Alt+F4, the taskbar) installs a waiting update too
        let me = Rc::downgrade(&c);
        ui.window().on_close_requested(move || { if let Some(c) = me.upgrade() { c.before_exit(); } slint::CloseRequestResponse::HideWindow });
    }
    on!(on_crash_send, |c| c.crash_send());
    on!(on_crash_save, |c| c.crash_save());
    on!(on_crash_close, |c| c.ui().set_crash_shown(false));
    on!(on_upd_check, |c| c.update_check(true));
    on!(on_upd_install, |c| c.update_install());
    on!(on_upd_auto_set, |c, v| { updater::set_auto(&c.opts.data_dir, v); c.ui().set_upd_auto(v); });
    on!(on_sm_open, |c| c.sm_open());
    on!(on_sm_close, |c| c.ui().set_sm_shown(false));
    on!(on_sm_run, |c| c.sm_run());
    on!(on_an_range, |c, i| c.an_range(i));
    on!(on_an_run, |c| c.an_run());
    on!(on_sm_range, |c, k| c.sm_range(k));
    on!(on_sm_export, |c, w| c.sm_export(w.as_str()));
    on!(on_print_report, |c| c.make_report_with(true));
    on!(on_share_whatsapp, |c| c.share_whatsapp());
    on!(on_share_email, |c| c.share_email());
    on!(on_em_save, |c| { c.email_save(); });
    on!(on_em_test, |c| c.email_test());
    on!(on_print_bill, |c| c.make_bill_with(true));
    on!(on_bill_prefs_save, |c| c.bill_prefs_save());
    on!(on_terms_accept, |c| {
        let r = c.st.borrow_mut().store.accept_terms();
        match r { Ok(()) => c.ui().set_terms_needed(false), Err(e) => c.fail(e) }
    });
    on!(on_terms_decline, |c| { c.before_exit(); let _ = c.ui().hide(); let _ = slint::quit_event_loop(); });
    on!(on_sign_in, |c| c.sign_in());
    on!(on_create_first_admin, |c| c.create_first_admin());
    on!(on_sign_out, |c| c.sign_out());
    on!(on_te_query_changed, |c| c.refresh_admin_tests());
    on!(on_te_new, |c| c.te_new());
    on!(on_te_open, |c, id| c.te_open(id as i64));
    on!(on_te_save, |c| c.te_save());
    on!(on_te_back, |c| c.te_back());
    on!(on_te_reset, |c| c.te_reset());
    on!(on_te_preview, |c| c.te_preview());
    on!(on_te_copy_changed, |c| c.te_copy_changed());
    on!(on_te_copy_pick, |c, i| c.te_copy_pick(i as usize));
    on!(on_te_open_text, |c, k| c.te_open_text(k));
    on!(on_ci_save, |c| c.ci_save());
    on!(on_ci_cancel, |c| c.ui().set_ci_shown(false));
    on!(on_fp_pick, |c, i| c.fp_load(i));
    on!(on_fp_new, |c| c.fp_clear());
    on!(on_fp_submit, |c| c.fp_submit());
    on!(on_row_delete, |c, i| c.row_delete(i as usize));
    on!(on_row_move, |c, i, d| c.row_move(i as usize, d));
    on!(on_row_select, |c, i, v| c.row_select(i as usize, v));
    on!(on_rows_delete_selected, |c| c.rows_delete_selected());
    on!(on_ar_open, |c, add| c.ar_open(add));
    on!(on_ar_add, |c| c.ar_add());
    on!(on_ar_remove, |c, i| c.ar_remove(i as usize));
    on!(on_ar_edit, |c, i, f, t| c.ar_edit(i as usize, f, t.as_str()));
    on!(on_ar_done, |c| c.ar_done());
    on!(on_ar_cancel, |c| c.ui().set_ar_shown(false));
    on!(on_pick_design, |c, i| c.pick_design(i));
    on!(on_prepare_choices, |c, i| c.prepare_choices(i as usize));
    on!(on_forgot_open, |c| c.set_forgot(true));
    on!(on_forgot_back, |c| c.set_forgot(false));
    on!(on_recover, |c| c.recover());
    on!(on_find_username, |c| c.find_username());
    on!(on_new_recovery_code, |c| c.ask_new_recovery());
    on!(on_rc_save_copy, |c| c.rc_save_copy());
    on!(on_rc_done, |c| c.ui().set_rc_shown(false));
    on!(on_ue_remove, |c| c.ue_remove());
    on!(on_confirm_yes_clicked, |c| c.confirm_yes());
    on!(on_confirm_no_clicked, |c| c.confirm_no());
    on!(on_match_edited, |c| c.match_edited());
    on!(on_pay_open, |c| c.pay_open());
    on!(on_pay_open_patient, |c| c.pay_open_patient());
    on!(on_pay_save, |c| c.pay_save());
    on!(on_pay_cancel, |c| c.ui().set_pay_shown(false));
    on!(on_mc_auto_toggle, |c| c.mc_auto_toggle());
    on!(on_mc_range_set, |c, i| c.mc_range_set(i));
    on!(on_mach_pick, |c, id| c.show_machine(id as i64));
    on!(on_mach_attach, |c, id| c.machine_attach(id as i64));
    on!(on_mach_dismiss, |c| c.machine_dismiss());
    on!(on_mach_quick, |c| c.machine_quick());
    on!(on_mach_edit, |c| c.machine_open_report(false));
    on!(on_mach_print, |c| c.machine_open_report(true));
    on!(on_print_full_bill, |c| c.print_full_bill());
    on!(on_ap_confirm, |c| c.approve_confirmed());
    on!(on_ap_cancel, |c| c.ui().set_ap_shown(false));
    on!(on_scan_edited, |c, t| c.scan_edited(t.as_str()));
    on!(on_scan_submit, |c| c.scan_submit());
    on!(on_scan_clear, |c| c.scan_clear());
    on!(on_open_patient, |c, id| { c.navigate(6); c.show_patient(id as i64); });
    on!(on_verdict_dismiss, |c| c.ui().set_verdict_kind(0));
    on!(on_ue_new, |c| c.ue_new());
    on!(on_ue_open, |c, id| c.ue_open(id as i64));
    on!(on_ue_save, |c| c.ue_save());
    on!(on_change_my_password, |c| c.change_my_password());
    on!(on_choose_letterhead, |c| c.choose_image("letterhead"));
    on!(on_choose_signature, |c| c.choose_image("signature"));
    on!(on_remove_letterhead, |c| c.remove_image("letterhead"));
    on!(on_remove_signature, |c| c.remove_image("signature"));
    on!(on_save_settings, |c| c.save_settings());
    on!(on_print_test_page, |c| c.print_test_page());
    on!(on_import_tests, |c| c.import_tests());
    on!(on_load_samples, |c| c.load_samples());
    on!(on_backup_now, |c| c.backup_now());
    on!(on_dismiss_toast, |c| c.hide_toast());
    on!(on_make_bill, |c| c.make_bill());
    on!(on_edit_patient, |c| c.edit_patient());
    on!(on_edit_save, |c| c.edit_save());
    on!(on_edit_cancel, |c| c.ui().set_edit_shown(false));
    on!(on_delete_report, |c| c.ask_delete());
    on!(on_doctor_edited, |c, t| c.doctor_edited(t.as_str()));
    on!(on_e_doctor_edited, |c, t| c.doctor_edited(t.as_str()));
    on!(on_choose_backup_folder, |c| c.choose_folder(false));
    on!(on_choose_cloud_folder, |c| c.choose_folder(true));
    on!(on_clear_cloud_folder, |c| c.ui().set_bk_cloud("".into()));
    on!(on_save_backup_settings, |c| { c.save_backup_settings(); });
    on!(on_restore_backup, |c| c.ask_restore(false));
    on!(on_restore_first_run, |c| c.ask_restore(true));
    on!(on_restore_deleted, |c, id| c.restore_deleted(id as i64));
    on!(on_help_edited, |c, t| c.refresh_help(t.as_str()));
    on!(on_make_problem_report, |c| c.problem_report());
    on!(on_open_github, |c| c.open_url("https://github.com/mstudio005"));
    on!(on_open_data_folder, |c| { let d = c.opts.data_dir.clone(); c.open_path(&d); });

    c.refresh_tests();
    c.refresh_orders();
    c.refresh_bill();
    c.refresh_home();
    c.load_settings();
    c.refresh_help("");
    c.doctor_edited("");
    c.show_lock_if_any();
    c.resume_saved_sign_in();
    c.show_crash_if_any();
    c.start_updates();
    let _ = crate::bridge::start();
    {
        // keep the maximize / restore icon right when the window is changed by the keyboard or by Windows
        let me = Rc::downgrade(&c);
        c.win_timer.start(TimerMode::Repeated, Duration::from_millis(300), move || { if let Some(c) = me.upgrade() { let u = c.ui(); let m = u.window().is_maximized(); if u.get_win_maxed() != m { u.set_win_maxed(m); } if !m && !u.window().is_minimized() { let (sz, sc) = (u.window().size(), u.window().scale_factor()); if sz.width > 400 { c.win_normal.set((sz.width as f32 / sc, sz.height as f32 / sc)); } } } });
    }
    c.machine_autostart();
    {
        let me = Rc::downgrade(&c);
        c.backup_timer.start(TimerMode::Repeated, Duration::from_secs(30 * 60), move || { if let Some(c) = me.upgrade() { c.auto_backup(); } });
    }
    c
}

fn row_of(s: &labdesk_core::OrderSummary) -> OrderRow {
    let due = s.total_paise - s.paid_paise;
    OrderRow { id: s.id as i32, code: s.patient_code.as_str().into(), name: s.patient_name.as_str().into(), age_gender: s.age_gender.as_str().into(),
        tests: s.test_names.join(", ").into(), status: s.status.as_str().into(), total: fmt::rupees(s.total_paise).into(),
        due: if due > 0 { fmt::rupees(due) } else { "—".into() }.into(), when: fmt::when(&s.created_at).into() }
}

fn lock_text(n: u64) -> String { format!("Too many wrong passwords. Please wait {n} seconds, then try again.") }

/// Turn a core error into a sentence a lab assistant can act on.
fn friendly(e: &Error) -> String {
    match e {
        Error::Locked(n) => lock_text(*n),
        Error::Forbidden(_) => "Your account is not allowed to do this. Ask the Admin if you need it.".into(),
        Error::Db(_) | Error::Json(_) => format!("Something went wrong with the data file. Please make a backup and tell your software provider. ({e})"),
        other => other.to_string(),
    }
}

fn thumbnail(bytes: &[u8], max_w: u32, max_h: u32) -> Option<slint::Image> {
    let img = image::load_from_memory(bytes).ok()?.thumbnail(max_w, max_h).to_rgba8();
    let (w, h) = (img.width(), img.height());
    Some(slint::Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(img.as_raw(), w, h)))
}

fn safe_name(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect::<String>().trim().to_string()
}

impl Ctl {
    fn ui(&self) -> AppWindow { self.ui.upgrade().expect("window alive") }
    /// The signed-in person. Everything the app records is attributed to them.
    fn actor(&self) -> Actor { self.st.borrow().user.as_ref().map_or_else(|| Actor::new("nobody", Role::Reception), |u| u.actor()) }

    // ---------- messages ----------
    /// A message that slides in from the top and fades away by itself. Problems stay longer.
    pub fn toast(&self, text: &str, bad: bool) {
        let ui = self.ui();
        ui.set_toast_text(text.into());
        ui.set_toast_bad(bad);
        ui.set_toast_shown(true);
        let w = self.ui.clone();
        self.toast_timer.start(TimerMode::SingleShot, Duration::from_millis(if bad { 7000 } else { 3800 }), move || { if let Some(u) = w.upgrade() { u.set_toast_shown(false); } });
    }
    fn hide_toast(&self) { self.ui().set_toast_shown(false); }
    fn fail(&self, e: Error) { self.toast(&friendly(&e), true); }

    // ---------- navigation ----------
    pub fn navigate(&self, page: i32) {
        let ui = self.ui();
        ui.set_page(page);
        ui.set_verdict_kind(0);
        match page {
            0 => self.refresh_home(),
            2 => self.refresh_orders(),
            6 => { self.refresh_patients(); }
            7 => { self.scan_clear(); }
            8 => { self.refresh_machine(); }
            9 => { self.an_range(self.ui().get_an_sel().max(0)); }
            3 => { self.load_settings(); self.refresh_admin_tests(); self.refresh_users(); self.refresh_data(); self.refresh_email(); }
            _ => {}
        }
    }
    // ---------- home ----------
    pub fn refresh_home(&self) {
        let ui = self.ui();
        self.bill_prefs_load();
        let (stats, orders, profile, has_lh, has_sig, n_tests) = {
            let st = self.st.borrow();
            let r = (|| -> labdesk_core::Result<_> { Ok((st.store.stats()?, st.store.orders("", 500)?, st.store.profile()?, st.store.asset("letterhead")?.is_some(), st.store.asset("signature")?.is_some(), st.store.tests()?.len())) })();
            match r { Ok(v) => v, Err(e) => { drop(st); return self.fail(e); } }
        };
        let ist = chrono::FixedOffset::east_opt(5 * 3600 + 1800).expect("valid offset");
        let now = chrono::Utc::now().with_timezone(&ist);
        use chrono::Timelike;
        ui.set_greeting(match now.hour() { 5..=11 => "Good morning", 12..=16 => "Good afternoon", 17..=21 => "Good evening", _ => "Hello" }.into());
        ui.set_today_text(now.format("%A, %-d %B").to_string().into());
        ui.set_lab_name(profile.name.as_str().into());
        ui.set_stat_patients(stats.patients_today.to_string().into());
        ui.set_stat_pending(stats.reports_pending.to_string().into());
        ui.set_stat_ready(stats.ready_to_deliver.to_string().into());
        ui.set_stat_due(fmt::rupees(stats.due_paise).into());
        ui.set_stat_collected(format!("{} collected today", fmt::rupees(stats.collected_today_paise)).into());
        ui.set_setup_name(!profile.name.trim().is_empty());
        ui.set_setup_letterhead(has_lh);
        ui.set_setup_signature(has_sig);
        ui.set_setup_tests(n_tests > 0);
        self.st.borrow().attention.set_vec(orders.iter().filter(|o| o.status != "Delivered").take(8).map(row_of).collect::<Vec<_>>());
    }

    // ---------- reports list ----------
    pub fn refresh_orders(&self) {
        let ui = self.ui();
        let mut q = ui.get_order_query().to_string();
        // A scanned report QR: show the patient and say whether the paper matches the lab's record.
        if q.contains("Report no:") { ui.set_order_query("".into()); if self.try_scan(&q) { return; } q.clear(); }
        let result = self.st.borrow().store.orders(&q, 500);
        match result {
            Ok(all) => {
                let st = self.st.borrow();
                let count = |s: &str| all.iter().filter(|o| s.is_empty() || o.status == s).count();
                st.filters.set_vec(FILTERS.iter().zip(STATUS).map(|(l, s)| SharedString::from(format!("{l}  {}", count(s)))).collect::<Vec<_>>());
                let want = STATUS[(ui.get_filter_index().max(0) as usize).min(4)];
                st.orders.set_vec(all.iter().filter(|o| want.is_empty() || o.status == want).map(row_of).collect::<Vec<_>>());
            }
            Err(e) => self.fail(e),
        }
    }

    // ---------- new patient ----------
    pub fn refresh_tests(&self) {
        let q = self.ui().get_test_query().to_string().to_lowercase();
        let result = self.st.borrow().store.tests();
        match result {
            Ok(all) => {
                let st = self.st.borrow();
                st.tests.set_vec(all.iter().filter(|t| !t.disabled && (q.is_empty() || t.name.to_lowercase().contains(&q) || t.code.to_lowercase().contains(&q))).take(400)
                    .map(|t| TestRow { id: t.id as i32, code: t.code.as_str().into(), name: t.name.as_str().into(), dept: t.department.as_str().into(), price: fmt::rupees(t.price_paise).into(), selected: st.selected.contains(&t.id) }).collect::<Vec<_>>());
            }
            Err(e) => self.fail(e),
        }
    }
    fn toggle_test(&self, id: i64) {
        { let mut st = self.st.borrow_mut(); if !st.selected.remove(&id) { st.selected.insert(id); } }
        self.ui().set_err_tests("".into());
        self.refresh_tests();
        self.refresh_bill();
    }
    fn subtotal(&self) -> i64 {
        let st = self.st.borrow();
        st.store.tests().map(|t| t.iter().filter(|t| st.selected.contains(&t.id)).map(|t| t.price_paise).sum()).unwrap_or(0)
    }
    fn refresh_bill(&self) {
        let ui = self.ui();
        let sub = self.subtotal();
        let disc = fmt::parse_rupees(&ui.get_f_discount()).unwrap_or(0);
        let paid = fmt::parse_rupees(&ui.get_f_paid()).unwrap_or(0);
        let total = (sub - disc).max(0);
        ui.set_selected_count(self.st.borrow().selected.len() as i32);
        ui.set_bill_subtotal(fmt::rupees(sub).into());
        ui.set_bill_total(fmt::rupees(total).into());
        ui.set_bill_due(fmt::rupees((total - paid).max(0)).into());
    }

    /// Check the form first and put each problem right beside the field it belongs to.
    fn register(&self) {
        let ui = self.ui();
        let mut bad = false;
        let mut flag = |cond: bool, set: &dyn Fn(&str), msg: &str| { if cond { set(msg); bad = true; } else { set(""); } };
        flag(ui.get_f_first().trim().is_empty(), &|m| ui.set_err_first(m.into()), "Please enter the patient's name");
        let age: Option<f64> = ui.get_f_age().trim().parse().ok().filter(|a: &f64| a.is_finite() && *a >= 0.0);
        flag(age.is_none(), &|m| ui.set_err_age(m.into()), if ui.get_f_age().trim().is_empty() { "Please enter the age, for example 34" } else { "The age must be a number, for example 34" });
        flag(ui.get_f_address().trim().is_empty(), &|m| ui.set_err_address(m.into()), "Please enter the patient's address");
        let email_err = labdesk_core::check_email(&ui.get_f_email()).err().map(|e| friendly(&e));
        flag(email_err.is_some(), &|m| ui.set_err_email(m.into()), email_err.as_deref().unwrap_or(""));
        let by_doctor = ui.get_f_ref() == 1;
        flag(by_doctor && ui.get_f_doctor().trim().is_empty(), &|m| ui.set_err_doctor(m.into()), "Please type the doctor's name, or choose Self");
        let ids: Vec<i64> = self.st.borrow().selected.iter().copied().collect();
        flag(ids.is_empty(), &|m| ui.set_err_tests(m.into()), "Choose at least one test from the list below");
        let (disc, paid) = (fmt::parse_rupees(&ui.get_f_discount()), fmt::parse_rupees(&ui.get_f_paid()));
        if disc.is_none() || paid.is_none() { self.toast("Please type the discount and the amount paid as plain numbers, for example 150 or 150.50", true); return; }
        if bad { self.toast("Please check the highlighted fields", true); return; }
        let (disc, paid, age) = (disc.unwrap_or(0), paid.unwrap_or(0), age.unwrap_or(0.0));
        // Is this person perhaps already registered? Same phone number, or exactly the same name.
        if self.st.borrow().linked.is_none() && !self.st.borrow().skip_match {
            let (f, l, ph) = (ui.get_f_first().to_string(), ui.get_f_last().to_string(), ui.get_f_phone().to_string());
            let found = self.st.borrow().store.possible_matches(&f, &l, &ph, 5).unwrap_or_default();
            let digits: String = ph.chars().filter(|c| c.is_ascii_digit()).collect();
            let same_name = format!("{} {}", f.trim(), l.trim()).trim().to_lowercase();
            if let Some(m) = found.iter().find(|m| (digits.len() >= 7 && m.phone.chars().filter(|c| c.is_ascii_digit()).collect::<String>().ends_with(&digits[digits.len().saturating_sub(10)..])) || m.name.to_lowercase().ends_with(&same_name)) {
                self.ask(Pending::SamePerson(m.id), "Is this person already registered?",
                    &format!("{}\n{} · {} · {}\n\nIf this is the same person, their saved details will be used and the visit is added to their record. If they are somebody else, choose NO.", m.name, m.patient_code, m.age_gender, if m.visits == 1 { "1 earlier visit".to_string() } else { format!("{} earlier visits", m.visits) }),
                    "YES, SAME PERSON", false);
                ui.set_confirm_no_text("NO, NEW PERSON".into());
                return;
            }
        }
        let new = NewOrder {
            patient: NewPatient { designation: String::new(), first_name: ui.get_f_first().into(), last_name: ui.get_f_last().into(), age,
                age_type: ["Year", "Month", "Day"][(ui.get_f_age_type().clamp(0, 2)) as usize].into(), gender: ["Male", "Female", "Other"][(ui.get_f_gender().clamp(0, 2)) as usize].into(),
                phone: ui.get_f_phone().into(), email: ui.get_f_email().into(), address: ui.get_f_address().into() },
            items: ids.iter().map(|id| OrderItem { test_id: *id, price_paise: None, discount_paise: 0 }).collect(),
            existing_patient_id: self.st.borrow().linked,
            referral_doctor: if by_doctor { ui.get_f_doctor().into() } else { String::new() },
            discount_paise: disc,
            payments: if paid > 0 { vec![(paid, ["Cash", "UPI", "Card", "Bank"][(ui.get_f_method().clamp(0, 3)) as usize].into(), String::new())] } else { vec![] },
            ..Default::default()
        };
        let actor = self.actor();
        let result = self.st.borrow_mut().store.register(&actor, new);
        match result {
            Ok(o) => {
                for s in [AppWindow::set_f_first as fn(&AppWindow, SharedString), AppWindow::set_f_last, AppWindow::set_f_age, AppWindow::set_f_phone, AppWindow::set_f_address, AppWindow::set_f_discount, AppWindow::set_f_paid, AppWindow::set_test_query, AppWindow::set_f_email, AppWindow::set_f_doctor] { s(&ui, "".into()); }
                ui.set_f_ref(0);
                self.clear_link();
                self.doctor_edited("");
                self.st.borrow_mut().selected.clear();
                self.refresh_tests(); self.refresh_bill(); self.refresh_orders();
                self.toast(&format!("{} is registered. Patient ID {}. You can enter the results now.", o.patient.first_name, o.patient.patient_code), false);
                self.show_order(o);
            }
            Err(Error::Rule(m)) if m.to_lowercase().contains("payment") => { self.toast(&m, true); }
            Err(e) => self.fail(e),
        }
    }

    // ---------- result sheet ----------
    pub fn open_order(&self, id: i64) {
        let result = self.st.borrow().store.order(id);
        match result { Ok(o) => self.show_order(o), Err(e) => self.fail(e) }
    }
    fn show_order(&self, o: Order) {
        {
            let mut st = self.st.borrow_mut();
            st.entries = o.tests.iter().flat_map(|t| t.results.iter().map(move |r| ((t.id, r.parameter_id.clone()), r.value.clone()))).collect();
            st.order = Some(o);
        }
        let ui = self.ui();
        ui.set_r_asking_reason(false);
        ui.set_page(4);
        self.rebuild_rows();
        self.refresh_header();
    }

    /// Compute what the sheet should show right now: typed values, recalculated formulas, flags.
    fn compute_rows(&self) -> Vec<ParamRow> {
        let st = self.st.borrow();
        let Some(o) = &st.order else { return vec![] };
        let mut rows = Vec::new();
        for t in &o.tests {
            rows.push(ParamRow { order_test_id: t.id as i32, param_id: "".into(), name: t.name.as_str().into(), unit: "".into(), range: "".into(), value: "".into(), flag: "".into(), kind: 2, formula: false, locked: true, status: t.status.as_str().into(), choice: false });
            let locked = t.is_approved() || o.status == "Delivered";
            let typed: Vec<ResultRow> = t.config.parameters.iter().map(|p| ResultRow {
                parameter_id: p.id.clone(), value: st.entries.get(&(t.id, p.id.clone())).cloned().unwrap_or_default(), flag: String::new(), range_text: None, comment: String::new(),
                manual: t.results.iter().any(|r| r.parameter_id == p.id && r.manual) }).collect();
            let shown = if locked { typed.clone() } else { calculate_results(&t.config.parameters, &typed) };
            for p in t.config.parameters.iter().filter(|p| !p.hidden) {
                if !p.is_result_field() {
                    rows.push(ParamRow { order_test_id: t.id as i32, param_id: p.id.as_str().into(), name: p.name.as_str().into(), unit: "".into(), range: "".into(), value: "".into(), flag: "".into(), kind: 1, formula: false, locked: true, status: "".into(), choice: false });
                    continue;
                }
                let value = shown.iter().find(|r| r.parameter_id == p.id).map(|r| r.value.clone()).unwrap_or_default();
                let saved = t.results.iter().find(|r| r.parameter_id == p.id).and_then(|r| r.range_text.as_deref());
                rows.push(ParamRow { order_test_id: t.id as i32, param_id: p.id.as_str().into(), name: p.name.as_str().into(), unit: p.unit.as_str().into(),
                    range: result_range(p, saved, &o.patient).into(), flag: flag_for(p, &value, &o.patient).into(), value: value.into(), kind: 0,
                    formula: !p.formula.is_empty() && !typed.iter().any(|r| r.parameter_id == p.id && r.manual), locked, status: "".into(), choice: p.field == "Dropdown" && !p.options.is_empty() && p.formula.is_empty() });
            }
        }
        rows
    }
    fn rebuild_rows(&self) { let rows = self.compute_rows(); self.st.borrow().params.set_vec(rows); }

    /// Fill the list shown when a "choice" result (like Colour) is clicked.
    fn prepare_choices(&self, idx: usize) {
        let opts: Vec<SharedString> = {
            let st = self.st.borrow();
            let key = st.params.row_data(idx).map(|r| (r.order_test_id as i64, r.param_id.to_string()));
            let found = key.and_then(|(tid, pid)| st.order.as_ref()?.tests.iter().find(|t| t.id == tid)?.config.parameters.iter().find(|p| p.id == pid).map(|p| p.options.clone()));
            found.unwrap_or_default().iter().map(|o| SharedString::from(o.as_str())).collect()
        };
        self.ui().set_choices(ModelRc::from(Rc::new(VecModel::from(opts))));
    }

    fn value_edited(&self, idx: usize, text: &str) {
        let key = { let st = self.st.borrow(); st.params.row_data(idx).map(|r| (r.order_test_id as i64, r.param_id.to_string())) };
        let Some(key) = key else { return };
        self.st.borrow_mut().entries.insert(key, text.to_string());
        // Live preview: update flags and calculated rows in place so the cursor never jumps.
        let new = self.compute_rows();
        let st = self.st.borrow();
        for (i, row) in new.into_iter().enumerate() {
            if i < st.params.row_count() && st.params.row_data(i).as_ref() != Some(&row) { st.params.set_row_data(i, row); }
        }
    }

    fn dirty_inputs(&self) -> Vec<ResultInput> {
        let st = self.st.borrow();
        let Some(o) = &st.order else { return vec![] };
        let mut out = Vec::new();
        for t in o.tests.iter().filter(|t| !t.is_approved()) {
            for p in t.config.parameters.iter().filter(|p| p.is_result_field() && p.formula.is_empty()) {
                let typed = st.entries.get(&(t.id, p.id.clone())).cloned().unwrap_or_default();
                let saved = t.results.iter().find(|r| r.parameter_id == p.id).map(|r| r.value.clone()).unwrap_or_default();
                if typed != saved { out.push(ResultInput { order_test_id: t.id, parameter_id: p.id.clone(), value: typed, comment: None, manual: false }); }
            }
        }
        out
    }

    /// Returns true if everything is saved (or there was nothing to save).
    fn save_draft(&self, announce: bool) -> bool {
        let inputs = self.dirty_inputs();
        let (id, version) = match &self.st.borrow().order { Some(o) => (o.id, o.version), None => return false };
        let actor = self.actor();
        let result = self.st.borrow_mut().store.save_results(&actor, id, version, &inputs, None);
        match result {
            Ok(o) => { if announce { self.toast(if inputs.is_empty() { "Nothing new to save" } else { "Results saved" }, false); } self.show_order(o); self.refresh_orders(); true }
            Err(e) => { self.fail(e); false }
        }
    }

    /// The APPROVE button: first show everything that is about to be approved and ask "are you sure?".
    fn approve(&self) {
        if !self.save_draft(false) { return; }
        let Some(o) = self.st.borrow().order.clone() else { return };
        let (rows, n) = sum_rows(&o, true);
        if n.results == 0 { self.toast("There are no results to approve yet. Type the results first.", true); return; }
        let ui = self.ui();
        let p = &o.patient;
        ui.set_ap_name(format!("{} {} {}", p.designation, p.first_name, p.last_name).split_whitespace().collect::<Vec<_>>().join(" ").into());
        ui.set_ap_meta(format!("ID {}  ·  {} {} / {}  ·  {}  ·  registered {}", p.patient_code, p.age, p.age_type.to_lowercase(), p.gender,
            if o.referral_name.trim().is_empty() { "came by themselves".to_string() } else { format!("sent by {}", o.referral_name) }, fmt::when_full(&o.created_at)).into());
        ui.set_ap_counts(format!("{} test{}  ·  {} result{}  ·  {} outside the normal range  ·  {} critical", n.tests, if n.tests == 1 { "" } else { "s" }, n.results, if n.results == 1 { "" } else { "s" }, n.abnormal, n.critical).into());
        let mut warn = Vec::new();
        if n.critical > 0 { warn.push(format!("{} CRITICAL value{}: please double-check before approving.", n.critical, if n.critical == 1 { "" } else { "s" })); }
        if n.blank > 0 { warn.push(format!("{} line{} left empty and will not be printed.", n.blank, if n.blank == 1 { "" } else { "s" })); }
        ui.set_ap_warn(warn.join("  ").into());
        let a = self.actor();
        ui.set_ap_by(format!("Approving as {} ({}). After approval the values are locked; changing them later needs a reason and is recorded.", a.name, match a.role { Role::Admin => "Admin", Role::Pathologist => "Pathologist", _ => "staff" }).into());
        self.st.borrow().ap_rows.set_vec(rows);
        ui.set_ap_shown(true);
    }
    fn approve_confirmed(&self) {
        self.ui().set_ap_shown(false);
        let (id, version) = { let st = self.st.borrow(); match &st.order { Some(o) => (o.id, o.version), None => return } };
        let actor = self.actor();
        let result = self.st.borrow_mut().store.approve(&actor, id, version, None);
        match result { Ok(o) => { self.toast("Approved. The report is ready: you can make the PDF now.", false); self.show_order(o); self.refresh_orders(); if self.print_after.replace(false) { self.make_report_with(true); } } Err(e) => self.fail(e) }
    }
    fn set_asking(&self, on: bool) { let ui = self.ui(); ui.set_r_asking_reason(on); if !on { ui.set_r_reason("".into()); } }
    fn confirm_correction(&self) {
        let (id, version) = { let st = self.st.borrow(); match &st.order { Some(o) => (o.id, o.version), None => return } };
        let reason = self.ui().get_r_reason().to_string();
        let actor = self.actor();
        let result = self.st.borrow_mut().store.correct(&actor, id, version, &reason, None);
        match result { Ok(o) => { self.set_asking(false); self.toast("The report is open for correction again", false); self.show_order(o); self.refresh_orders(); } Err(e) => self.fail(e) }
    }
    fn deliver(&self) {
        let (id, version) = { let st = self.st.borrow(); match &st.order { Some(o) => (o.id, o.version), None => return } };
        let actor = self.actor();
        let result = self.st.borrow_mut().store.mark_delivered(&actor, id, version);
        match result { Ok(o) => { self.toast("Marked as delivered. This report is now locked and cannot be changed.", false); self.show_order(o); self.refresh_orders(); } Err(e) => self.fail(e) }
    }

    fn refresh_header(&self) {
        let ui = self.ui();
        let st = self.st.borrow();
        let Some(o) = &st.order else { return };
        let role = st.user.as_ref().map_or(Role::Reception, |u| u.role);
        let any_open = o.tests.iter().any(|t| !t.is_approved());
        let any_approved = o.tests.iter().any(|t| t.is_approved());
        let delivered = o.status == "Delivered";
        ui.set_r_title(format!("{} {} {}", o.patient.designation, o.patient.first_name, o.patient.last_name).trim().into());
        ui.set_r_sub(format!("{} · {} {} · {} · {}", o.patient.patient_code, o.patient.age, o.patient.age_type.to_lowercase(), o.patient.gender, if o.referral_name.is_empty() { "self".to_string() } else { format!("ref. {}", o.referral_name) }).to_uppercase().into());
        ui.set_r_status(o.status.as_str().into());
        ui.set_r_money(format!("BILL {} · PAID {} · DUE {}", fmt::rupees(o.total_paise), fmt::rupees(o.paid_paise()), fmt::rupees(o.due_paise().max(0))).into());
        let can_enter = matches!(role, Role::Admin | Role::Technician | Role::Pathologist);
        let can_approve = matches!(role, Role::Admin | Role::Pathologist);
        ui.set_r_can_save(can_enter && any_open && !delivered);
        ui.set_r_can_approve(can_approve && any_open && !delivered);
        ui.set_r_can_correct(can_approve && any_approved && !delivered);
        ui.set_r_can_deliver(matches!(role, Role::Admin | Role::Reception) && o.status == "Completed");
        ui.set_r_can_print(any_approved);
        ui.set_r_pay_text(if o.payments.is_empty() { "No payment has been received for this bill yet.".to_string() } else {
            format!("Payments received for this bill: {}", o.payments.iter().map(|p| format!("{} {} {}", fmt::when_full(&p.created_at), p.method, fmt::rupees(p.amount_paise))).collect::<Vec<_>>().join("  ·  ")) }.into());
        ui.set_r_can_edit(matches!(role, Role::Admin | Role::Reception));
        ui.set_r_can_delete(role == Role::Admin && !delivered);
        ui.set_r_can_bill(matches!(role, Role::Admin | Role::Reception));
        ui.set_r_can_pay(matches!(role, Role::Admin | Role::Reception) && o.due_paise() > 0 && o.deleted_at.is_none());
        ui.set_r_lock_note(if delivered { "This report was delivered, so it is locked and cannot be changed." }
            else if o.status == "Completed" { "Approved. After you hand the report over, press MARK DELIVERED." }
            else if !can_enter { "Results are entered by the Technician or the Doctor. You can register patients, take payments and hand over reports." }
            else if !can_approve && any_open { "When all results are in, press SAVE. The doctor then checks and presses APPROVE." }
            else { "" }.into());
    }

    // ---------- report PDF ----------
    fn save_pdf(&self, file_stem: &str, pdf: &[u8]) -> Result<PathBuf, String> {
        let dir = self.opts.docs_dir.join("Reports");
        std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create the folder {}: {e}", dir.display()))?;
        let first = dir.join(format!("{file_stem}.pdf"));
        // If the earlier copy is still open in a PDF viewer Windows refuses to overwrite it, so save beside it.
        let path = match std::fs::write(&first, pdf) {
            Ok(()) => first,
            Err(_) => { let alt = dir.join(format!("{file_stem} ({}).pdf", chrono::Local::now().format("%H%M%S"))); std::fs::write(&alt, pdf).map_err(|e| format!("Could not save the PDF: {e}"))?; alt }
        };
        Ok(path)
    }
    /// Send the saved PDF to the default printer. The file stays saved; if printing fails it is opened instead.
    fn print_saved(&self, path: &Path, what: &str) {
        if !self.opts.open_files { self.toast(&format!("{what} saved in {}", path.parent().map(|p| p.display().to_string()).unwrap_or_default()), false); return; }
        match crate::printing::print_pdf(path) {
            Ok(()) => self.toast(&format!("{what} sent to the printer"), false),
            Err(m) => { let _ = open::that(path); self.toast(&format!("{m} The {} is open: print it from there.", what.to_lowercase()), true); }
        }
    }
    fn show_pdf(&self, path: &Path, what: &str) {
        if self.opts.open_files { let _ = open::that(path); }
        self.toast(&format!("{what} saved in {}", path.parent().map(|p| p.display().to_string()).unwrap_or_default()), false);
    }
    // ---------- trend of one result over visits ----------
    fn trend_load(&self, patient_id: i64) {
        let list = self.st.borrow().store.trends(patient_id).unwrap_or_default();
        let names: Vec<SharedString> = list.iter().map(|t| SharedString::from(t.name.as_str())).collect();
        let first = list.first().map(|t| t.name.clone());
        self.st.borrow().tr_names.set_vec(names);
        self.st.borrow_mut().trends = list;
        match first { Some(n) => self.trend_show(&n), None => { self.st.borrow().tr_points.set_vec(vec![]); self.ui().set_tr_pick("".into()); } }
    }
    fn trend_show(&self, name: &str) {
        let ui = self.ui();
        let (w, h) = (640.0f32, 160.0f32);
        let st = self.st.borrow();
        let Some(t) = st.trends.iter().find(|t| t.name == name) else { return };
        let vals: Vec<f64> = t.points.iter().map(|p| p.value).chain(t.low).chain(t.high).collect();
        let (mut lo, mut hi) = (vals.iter().cloned().fold(f64::MAX, f64::min), vals.iter().cloned().fold(f64::MIN, f64::max));
        if (hi - lo).abs() < 1e-9 { lo -= 1.0; hi += 1.0; }
        let pad = (hi - lo) * 0.12; lo -= pad; hi += pad;
        let y = |v: f64| -> f32 { (h as f64 - 12.0 - (v - lo) / (hi - lo) * (h as f64 - 24.0) + 6.0) as f32 };
        let n = t.points.len();
        let x = |i: usize| -> f32 { if n == 1 { w / 2.0 } else { 50.0 + i as f32 * (w - 100.0) / (n as f32 - 1.0) } };
        let mut path = String::new();
        let mut pts = Vec::new();
        for (i, p) in t.points.iter().enumerate() {
            path.push_str(&format!("{} {:.1} {:.1} ", if i == 0 { "M" } else { "L" }, x(i), y(p.value)));
            pts.push(TrPoint { x: x(i), y: y(p.value), label: format!("{}\n{}", p.value, fmt::day(&p.when)).into(), bad: !p.flag.is_empty() });
        }
        let (top, bottom) = (t.high.map(|v| y(v)).unwrap_or(6.0).max(0.0), t.low.map(|v| y(v)).unwrap_or(h - 6.0).min(h));
        ui.set_tr_band_top(top); ui.set_tr_band_h((bottom - top).max(0.0));
        ui.set_tr_pick(name.into()); ui.set_tr_unit(t.unit.as_str().into());
        ui.set_tr_range(match (t.low, t.high) { (Some(l), Some(h)) => format!("Normal {l} - {h} {}  (green band)", t.unit), (Some(l), None) => format!("Normal at least {l} {}", t.unit), (None, Some(h)) => format!("Normal up to {h} {}", t.unit), _ => t.unit.clone() }.into());
        ui.set_tr_path(path.trim().into());
        st.tr_points.set_vec(pts);
    }

    // ---------- summary and export ----------
    fn ist_today() -> chrono::NaiveDate { chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(5 * 3600 + 1800).expect("valid offset")).date_naive() }
    fn sm_open(&self) {
        let ui = self.ui();
        self.sm_range(0);
        ui.set_sm_shown(true);
    }
    // ---------- email ----------
    fn refresh_email(&self) {
        let ui = self.ui();
        let st = self.st.borrow();
        if !st.user.as_ref().is_some_and(|u| u.role == Role::Admin) { return; }
        if let Ok(c) = st.store.email_settings() {
            ui.set_em_on(c.enabled as i32);
            ui.set_em_provider(match c.provider.as_str() { "proton" => 0, "gmail" => 1, "outlook" => 2, _ => 3 });
            ui.set_em_host(c.host.as_str().into()); ui.set_em_port(c.port.to_string().into());
            ui.set_em_user(c.username.as_str().into()); ui.set_em_pass(c.password.as_str().into()); ui.set_em_name(c.from_name.as_str().into());
        }
        ui.set_err_em("".into());
    }
    /// Returns true when saved.
    fn email_save(&self) -> bool {
        let ui = self.ui();
        ui.set_err_em("".into());
        let provider = ["proton", "gmail", "outlook", "custom"][ui.get_em_provider().clamp(0, 3) as usize];
        let port: u16 = ui.get_em_port().trim().parse().unwrap_or(0);
        let cfg = labdesk_core::EmailSettings { enabled: ui.get_em_on() == 1, provider: provider.into(), host: ui.get_em_host().into(), port, username: ui.get_em_user().into(), password: ui.get_em_pass().into(), from_name: ui.get_em_name().into() };
        let actor = self.actor();
        let r = self.st.borrow_mut().store.save_email_settings(&actor, &cfg);
        match r {
            Ok(()) => { self.refresh_email(); self.toast(if cfg.enabled { "Email settings saved. The EMAIL button is now on." } else { "Email settings saved. Email is off." }, false); true }
            Err(e) => { ui.set_err_em(friendly(&e).into()); false }
        }
    }
    fn email_test(&self) {
        if !self.email_save() { return; }
        let cfg = self.st.borrow().store.email_settings().unwrap_or_default();
        if !cfg.ready() { return self.toast("Turn Email on first, then send the test.", true); }
        let to = cfg.username.clone();
        let lab = self.st.borrow().store.profile().map(|p| p.name).unwrap_or_default();
        let msg = match crate::mailer::build_message(&cfg, &lab, &to, "LabDesk test email", "This is a test email from LabDesk. If you can read it, sending reports by email will work.", None) { Ok(m) => m, Err(m) => return self.toast(&m, true) };
        self.mail_spawn(cfg, msg, 0, to);
    }
    /// Run the sending in a thread, so the screen does not freeze while the mail server answers.
    fn mail_spawn(&self, cfg: labdesk_core::EmailSettings, msg: lettre::Message, order: i64, to: String) {
        if self.mail_rx.borrow().is_some() { return self.toast("An email is still being sent. Please wait a moment.", true); }
        let ui = self.ui();
        ui.set_em_busy(true);
        self.toast("Sending the email…", false);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || { let result = crate::mailer::send(&cfg, &msg); let _ = tx.send(MailDone { result, order, to }); });
        *self.mail_rx.borrow_mut() = Some(rx);
        let me = self.me.borrow().clone();
        self.mail_timer.start(TimerMode::Repeated, Duration::from_millis(400), move || { if let Some(c) = me.upgrade() { c.mail_poll(); } });
    }
    fn mail_poll(&self) {
        use std::sync::mpsc::TryRecvError;
        let got = match self.mail_rx.borrow().as_ref().map(|r| r.try_recv()) {
            Some(Ok(d)) => Some(d),
            Some(Err(TryRecvError::Disconnected)) => Some(MailDone { result: Err("Sending stopped unexpectedly.".into()), order: -1, to: String::new() }),
            _ => None,
        };
        let Some(d) = got else { return };
        self.mail_timer.stop();
        *self.mail_rx.borrow_mut() = None;
        self.ui().set_em_busy(false);
        if d.order > 0 { let actor = self.actor(); let _ = self.st.borrow_mut().store.log_email(&actor, d.order, &d.to, d.result.is_ok()); }
        match d.result {
            Ok(()) => self.toast(&if d.order > 0 { format!("The report was emailed to {}", d.to) } else { format!("Test email sent to {}. Check that inbox.", d.to) }, false),
            Err(m) => self.toast(&m, true),
        }
    }
    /// EMAIL on a report: check everything, then ask "send to this address?" before anything leaves the computer.
    fn share_email(&self) {
        let Some(o) = self.st.borrow().order.clone() else { return };
        let cfg = self.st.borrow().store.email_settings().unwrap_or_default();
        if !cfg.ready() { return self.toast(&if self.ui().get_is_admin() { "Email is not turned on. Turn it on in Settings > Email.".to_string() } else { "Email is not turned on. Ask the Admin to turn it on in Settings > Email.".to_string() }, true); }
        let to = o.patient.email.trim().to_string();
        if !labdesk_core::valid_address(&to) { return self.toast("This patient has no valid email address. Use EDIT PATIENT to add it.", true); }
        let name = format!("{} {} {}", o.patient.designation, o.patient.first_name, o.patient.last_name).split_whitespace().collect::<Vec<_>>().join(" ");
        self.ask(Pending::SendEmail(o.id), "Send this report by email?", &format!("The report of {name} will be emailed, with the PDF attached, to:\n\n{to}\n\nCheck that this is the address the patient gave you. It goes through the mail server {} and cannot be taken back.", cfg.host), "YES, SEND", false);
    }
    fn do_send_email(&self, id: i64) {
        let Some(o) = self.st.borrow().order.clone() else { return };
        if o.id != id { return; }
        let cfg = self.st.borrow().store.email_settings().unwrap_or_default();
        if !cfg.ready() { return self.toast("Email is not turned on. Turn it on in Settings > Email.", true); }
        let to = o.patient.email.trim().to_string();
        let lab = self.st.borrow().store.profile().map(|p| p.name).unwrap_or_default();
        let name = format!("{} {}", o.patient.designation, o.patient.first_name).split_whitespace().collect::<Vec<_>>().join(" ");
        let lab_shown = if lab.trim().is_empty() { "our lab".to_string() } else { lab.trim().to_string() };
        let letterhead = self.ui().get_use_letterhead();
        let actor = self.actor();
        let result = self.st.borrow_mut().store.report_pdf(&actor, o.id, letterhead);
        let pdf = match result { Ok(r) => r.pdf, Err(e) => return self.fail(e) };
        let stem = safe_name(&format!("{} {} {}", o.patient.patient_code, o.patient.first_name, o.patient.last_name));
        if let Err(m) = self.save_pdf(&stem, &pdf) { return self.toast(&m, true); }
        let body = format!("Dear {name},\n\nYour report from {lab_shown} is attached to this email as a PDF.\n\nThis is an automatic message. Please do not share your report with anyone you do not trust.\n\n{lab_shown}");
        let file = format!("{stem}.pdf");
        let msg = match crate::mailer::build_message(&cfg, &lab, &to, &format!("Your report from {lab_shown}"), &body, Some((&file, &pdf))) { Ok(m) => m, Err(m) => return self.toast(&m, true) };
        self.mail_spawn(cfg, msg, o.id, to);
    }
    // ---------- analytics ----------
    fn an_range(&self, kind: i32) {
        use chrono::Datelike;
        let ui = self.ui();
        let t = Self::ist_today();
        let from = match kind { 0 => t - chrono::Duration::days(6), 2 => t.with_day(1).unwrap_or(t), 3 => t.with_day(1).and_then(|d| d.with_month(1)).unwrap_or(t), _ => t - chrono::Duration::days(29) };
        ui.set_an_sel(kind);
        ui.set_an_from(from.format("%Y-%m-%d").to_string().into()); ui.set_an_to(t.format("%Y-%m-%d").to_string().into());
        self.an_run();
    }
    fn an_run(&self) {
        use labdesk_core::analytics::Totals;
        let ui = self.ui();
        let actor = self.actor();
        // today against yesterday
        let t = Self::ist_today();
        let day = |d: chrono::NaiveDate| d.format("%Y-%m-%d").to_string();
        let today = self.st.borrow().store.analytics(&actor, &day(t), &day(t));
        let ykey = day(t - chrono::Duration::days(1));
        let yest = self.st.borrow().store.analytics(&actor, &ykey, &ykey);
        if let (Ok(a), Ok(y)) = (&today, &yest) {
            let k = |label: &str, v: String, now: i64, before: i64, good_up: bool| {
                let money = v.contains('₹');
                let (change, tone) = Self::an_change(now, before, good_up, money);
                AnKpi { label: label.into(), value: v.into(), change: change.into(), tone }
            };
            let (c, p) = (&a.totals, &y.totals);
            let v = vec![
                k("PATIENTS TODAY", c.visits.to_string(), c.visits, p.visits, true),
                k("TESTS TODAY", c.tests.to_string(), c.tests, p.tests, true),
                k("BILLED TODAY", fmt::rupees(c.billed_paise), c.billed_paise, p.billed_paise, true),
                k("RECEIVED TODAY", fmt::rupees(c.received_paise), c.received_paise, p.received_paise, true),
                AnKpi { label: "REPORTS WAITING".into(), value: a.waiting_reports.to_string().into(), change: "not yet approved".into(), tone: 0 },
            ];
            ui.set_an_today(ModelRc::new(VecModel::from(v)));
        }
        let (from, to) = (ui.get_an_from().to_string(), ui.get_an_to().to_string());
        let r = self.st.borrow().store.analytics(&actor, &from, &to);
        let a = match r { Ok(a) => a, Err(e) => { ui.set_err_an(e.to_string().into()); return; } };
        ui.set_err_an("".into());
        let (c, p): (&Totals, &Totals) = (&a.totals, &a.previous);
        let k = |label: &str, v: String, now: i64, before: i64, good_up: bool| {
                let money = v.contains('₹');
            let (change, tone) = Self::an_change(now, before, good_up, money);
            AnKpi { label: label.into(), value: v.into(), change: change.into(), tone }
        };
        let avg_bill = |t: &Totals| if t.visits > 0 { t.billed_paise / t.visits } else { 0 };
        let turn = |m: Option<i64>| match m { None => "none yet".to_string(), Some(m) if m < 60 => format!("{m} min"), Some(m) => format!("{}h {}m", m / 60, m % 60) };
        ui.set_an_kpis(ModelRc::new(VecModel::from(vec![
            k("PATIENTS", c.visits.to_string(), c.visits, p.visits, true),
            k("TESTS", c.tests.to_string(), c.tests, p.tests, true),
            k("BILLED", fmt::rupees(c.billed_paise), c.billed_paise, p.billed_paise, true),
            k("RECEIVED", fmt::rupees(c.received_paise), c.received_paise, p.received_paise, true),
            k("STILL UNPAID", fmt::rupees(c.due_paise), c.due_paise, p.due_paise, false),
            k("AVERAGE BILL", fmt::rupees(avg_bill(c)), avg_bill(c), avg_bill(p), true),
            AnKpi { label: "AVG REPORT TIME".into(), value: turn(c.avg_turnaround_min).into(), change: format!("before: {}", turn(p.avg_turnaround_min)).into(), tone: 0 },
        ])));
        ui.set_an_title(format!("{} to {}  ({} day{}). Arrows compare with the {} days before: {}.", a.from, a.to, a.days, if a.days == 1 { "" } else { "s" }, a.days, a.previous_label).into());
        ui.set_an_people(format!("{} new patient{} and {} returning patient{} in these days.", c.new_patients, if c.new_patients == 1 { "" } else { "s" }, c.returning_patients, if c.returning_patients == 1 { "" } else { "s" }).into());
        // charts
        let unit = if a.monthly { "month" } else { "day" };
        let maxm = a.buckets.iter().map(|b| b.billed_paise.max(b.received_paise)).max().unwrap_or(0).max(1) as f32;
        let maxv = a.buckets.iter().map(|b| b.tests.max(b.visits)).max().unwrap_or(0).max(1) as f32;
        ui.set_an_money(ModelRc::new(VecModel::from(a.buckets.iter().map(|b| AnBar { label: b.label.as_str().into(), a: b.billed_paise.max(b.received_paise) as f32 / maxm, b: b.billed_paise.min(b.received_paise) as f32 / maxm }).collect::<Vec<_>>())));
        ui.set_an_visits(ModelRc::new(VecModel::from(a.buckets.iter().map(|b| AnBar { label: b.label.as_str().into(), a: b.tests.max(b.visits) as f32 / maxv, b: b.tests.min(b.visits) as f32 / maxv }).collect::<Vec<_>>())));
        ui.set_an_money_top(format!("Tallest bar = {}", fmt::rupees(maxm as i64)).into());
        ui.set_an_visits_top(format!("Tallest bar = {} tests", maxv as i64).into());
        let best_m = a.buckets.iter().max_by_key(|b| b.received_paise).filter(|b| b.received_paise > 0);
        ui.set_an_money_note(match best_m { Some(b) => format!("Most money in one {unit}: {} on {}.", fmt::rupees(b.received_paise), b.label), None => format!("No money received in these {unit}s.") }.into());
        let best_v = a.buckets.iter().max_by_key(|b| b.visits).filter(|b| b.visits > 0);
        ui.set_an_visits_note(match best_v { Some(b) => format!("Busiest {unit}: {} with {} patient{}.", b.label, b.visits, if b.visits == 1 { "" } else { "s" }), None => format!("No patients in these {unit}s.") }.into());
        let hmax = a.hours.iter().copied().max().unwrap_or(0).max(1);
        ui.set_an_hours(ModelRc::new(VecModel::from((0..24).map(|h| AnBar { label: format!("{h:02}:00").into(), a: a.hours[h] as f32 / hmax as f32, b: a.hours[h] as f32 / hmax as f32 }).collect::<Vec<_>>())));
        let peak = (0..24).max_by_key(|h| a.hours[*h]).filter(|h| a.hours[*h] > 0);
        ui.set_an_hours_note(match peak { Some(h) => format!("Busiest hour: {:02}:00 to {:02}:00 ({} patients).", h, (h + 1) % 24, a.hours[h]), None => "No patients yet.".to_string() }.into());
        // lists
        let rows = |v: Vec<(String, String, i64)>| { let m = v.iter().map(|x| x.2).max().unwrap_or(0).max(1) as f32; ModelRc::new(VecModel::from(v.into_iter().map(|(l, val, n)| AnRow { label: l.into(), value: val.into(), frac: n as f32 / m }).collect::<Vec<_>>())) };
        ui.set_an_methods(rows(a.by_method.iter().map(|(m, v)| (m.clone(), fmt::rupees(*v), *v)).collect()));
        ui.set_an_depts(rows(a.by_dept.iter().map(|d| (d.name.clone(), format!("{} tests · {}", d.tests, fmt::rupees(d.value_paise)), d.tests)).collect()));
        ui.set_an_tests(rows(a.top_tests.iter().map(|(n, c)| (n.clone(), c.to_string(), *c)).collect()));
        ui.set_an_docs(rows(a.referrals.iter().take(10).map(|r| (r.name.clone(), format!("{} · {}", r.visits, fmt::rupees(r.billed_paise)), r.billed_paise)).collect()));
    }
    /// "up 12% (from 40)" with a colour: 1 good, 2 bad, 0 neutral.
    fn an_change(now: i64, before: i64, good_up: bool, money: bool) -> (String, i32) {
        let show = |v: i64| if money { fmt::rupees(v) } else { v.to_string() };
        if now == before { return ("same as before".into(), 0); }
        let up = now > before;
        let tone = if up == good_up { 1 } else { 2 };
        let arrow = if up { "▲" } else { "▼" };
        if before == 0 { return (format!("{arrow} was {} before", show(0)), tone); }
        let pct = ((now - before).abs() as f64 / before.abs() as f64 * 100.0).round() as i64;
        (format!("{arrow} {pct}% (was {})", show(before)), tone)
    }
    fn sm_range(&self, kind: i32) {
        use chrono::Datelike;
        let ui = self.ui();
        let t = Self::ist_today();
        let from = match kind { 1 => t - chrono::Duration::days(6), 2 => t.with_day(1).unwrap_or(t), _ => t };
        ui.set_sm_from(from.format("%Y-%m-%d").to_string().into()); ui.set_sm_to(t.format("%Y-%m-%d").to_string().into());
        self.sm_run();
    }
    fn sm_run(&self) {
        let ui = self.ui();
        let (from, to) = (ui.get_sm_from().to_string(), ui.get_sm_to().to_string());
        let actor = self.actor();
        let r = self.st.borrow().store.summary(&actor, &from, &to);
        match r {
            Ok(s) => {
                ui.set_err_sm("".into());
                let mut rows = vec![SmRow { kind: 0, a: "MONEY".into(), b: "".into() }];
                let line = |a: &str, b: String| SmRow { kind: 1, a: a.into(), b: b.into() };
                rows.push(line("Visits (reports registered)", s.visits.to_string()));
                rows.push(line("Billed", fmt::rupees(s.billed_paise)));
                rows.push(line("Discount given", fmt::rupees(s.discount_paise)));
                rows.push(line("Money received in these days", fmt::rupees(s.received_paise)));
                for (m, a) in &s.by_method { rows.push(line(&format!("    {m}"), fmt::rupees(*a))); }
                rows.push(line("Still unpaid on these visits", fmt::rupees(s.due_paise)));
                rows.push(SmRow { kind: 0, a: "BY REFERRING DOCTOR".into(), b: "".into() });
                for r in &s.referrals { rows.push(line(&r.name, format!("{} visit{}  ·  billed {}  ·  received {}", r.visits, if r.visits == 1 { "" } else { "s" }, fmt::rupees(r.billed_paise), fmt::rupees(r.received_paise)))); }
                rows.push(SmRow { kind: 0, a: "TESTS DONE".into(), b: "".into() });
                for (n, c) in s.tests.iter().take(30) { rows.push(line(n, c.to_string())); }
                self.st.borrow().sm_rows.set_vec(rows);
            }
            Err(e) => { ui.set_err_sm(e.to_string().into()); self.st.borrow().sm_rows.set_vec(vec![]); }
        }
    }
    fn sm_export(&self, what: &str) {
        let ui = self.ui();
        let (from, to) = (ui.get_sm_from().to_string(), ui.get_sm_to().to_string());
        let actor = self.actor();
        let r = self.st.borrow().store.export_csv(&actor, what, &from, &to);
        match r {
            Ok(text) => {
                let dir = self.opts.docs_dir.join("Exports");
                if let Err(e) = std::fs::create_dir_all(&dir) { return self.toast(&format!("Could not create the folder {}: {e}", dir.display()), true); }
                let stem = format!("{what} {from} to {to}");
                let mut path = dir.join(format!("{stem}.csv"));
                if std::fs::write(&path, &text).is_err() { path = dir.join(format!("{stem} ({}).csv", chrono::Local::now().format("%H%M%S"))); if let Err(e) = std::fs::write(&path, &text) { return self.toast(&format!("Could not save the file: {e}"), true); } }
                if self.opts.open_files { let _ = open::that(&path); }
                self.toast(&format!("Saved in {}", dir.display()), false);
            }
            Err(e) => self.fail(e),
        }
    }
    /// Save the report PDF, put the file on the clipboard and open the patient's WhatsApp chat with a short message.
    fn share_whatsapp(&self) {
        let Some(o) = self.st.borrow().order.clone() else { return };
        let phone = o.patient.phone.clone();
        let lab = self.st.borrow().store.profile().map(|p| p.name).unwrap_or_default();
        let name = format!("{} {}", o.patient.designation, o.patient.first_name).split_whitespace().collect::<Vec<_>>().join(" ");
        let text = format!("Dear {name}, your report from {} is ready. Please find it attached.", if lab.trim().is_empty() { "our lab" } else { lab.trim() });
        if crate::printing::whatsapp_url(&phone, &text).is_none() { return self.toast("This patient has no valid phone number. Use EDIT PATIENT to add it.", true) }
        let letterhead = self.ui().get_use_letterhead();
        let actor = self.actor();
        let result = self.st.borrow_mut().store.report_pdf(&actor, o.id, letterhead);
        match result {
            Ok(r) => match self.save_pdf(&safe_name(&format!("{} {} {}", o.patient.patient_code, o.patient.first_name, o.patient.last_name)), &r.pdf) {
                Ok(p) => {
                    let copied = self.opts.open_files && crate::printing::copy_file_to_clipboard(&p);
                    // The LabDesk Bridge extension (if installed) fetches the PDF with this one-time code and attaches it.
                    let share = crate::bridge::global().offer(&format!("{}.pdf", safe_name(&format!("{} {} {}", o.patient.patient_code, o.patient.first_name, o.patient.last_name))), r.pdf.clone());
                    let url = crate::printing::whatsapp_web_url(&phone, &text, &share).unwrap_or_default();
                    if self.opts.open_files { let _ = open::that(&url); }
                    self.toast(&if copied { "WhatsApp Web is opening. The LabDesk Bridge extension attaches the report by itself. Without it, press Ctrl+V in the chat. Then press Send.".to_string() } else { format!("WhatsApp Web is opening. Attach the file from {}", p.display()) }, false);
                }
                Err(m) => self.toast(&m, true),
            },
            Err(e) => self.fail(e),
        }
    }
    fn make_report(&self) { self.make_report_with(false); }
    fn make_report_with(&self, print: bool) {
        let id = match &self.st.borrow().order { Some(o) => o.id, None => return };
        let letterhead = self.ui().get_use_letterhead();
        let actor = self.actor();
        let result = self.st.borrow_mut().store.report_pdf(&actor, id, letterhead);
        match result {
            Ok(r) => {
                let (code, name) = { let st = self.st.borrow(); let o = st.order.as_ref().expect("order open"); (o.patient.patient_code.clone(), format!("{} {}", o.patient.first_name, o.patient.last_name)) };
                match self.save_pdf(&safe_name(&format!("{code} {name}")), &r.pdf) { Ok(p) => if print { self.print_saved(&p, "Report") } else { self.show_pdf(&p, "Report") }, Err(m) => self.toast(&m, true) }
            }
            Err(e) => self.fail(e),
        }
    }
    fn print_test_page(&self) {
        let letterhead = true;
        let result = self.st.borrow().store.test_page_pdf(letterhead);
        match result {
            Ok(r) => match self.save_pdf("Test page", &r.pdf) { Ok(p) => self.show_pdf(&p, "Test page"), Err(m) => self.toast(&m, true) },
            Err(e) => self.fail(e),
        }
    }

    // ---------- settings ----------
    pub fn load_settings(&self) {
        let ui = self.ui();
        let st = self.st.borrow();
        let (p, lh, sig, n) = match (|| -> labdesk_core::Result<_> { Ok((st.store.profile()?, st.store.asset("letterhead")?, st.store.asset("signature")?, st.store.tests()?.len())) })() { Ok(v) => v, Err(_) => return };
        ui.set_s_name(p.name.as_str().into()); ui.set_s_address(p.address.as_str().into()); ui.set_s_phone(p.phone.as_str().into()); ui.set_s_email(p.email.as_str().into());
        ui.set_s_signatory(p.signatory.as_str().into()); ui.set_s_qualification(p.qualification.as_str().into()); ui.set_s_design(p.header_design as i32); ui.set_s_size(match p.text_size { ..=84 => 0, 85..=94 => 1, 95..=104 => 2, _ => 3 });
        let mm = |v: f32| if v.fract() == 0.0 { format!("{}", v as i32) } else { format!("{v}") };
        ui.set_s_top(mm(p.top_mm).into()); ui.set_s_bottom(mm(p.bottom_mm).into()); ui.set_s_left(mm(p.left_mm).into()); ui.set_s_right(mm(p.right_mm).into());
        ui.set_has_letterhead(lh.is_some()); ui.set_has_signature(sig.is_some());
        ui.set_has_recovery(st.store.has_recovery_code().unwrap_or(false));
        if let Some(i) = lh.as_deref().and_then(|b| thumbnail(b, 330, 470)) { ui.set_letterhead_img(i); }
        if let Some(i) = sig.as_deref().and_then(|b| thumbnail(b, 330, 120)) { ui.set_signature_img(i); }
        ui.set_test_count_text(if n == 0 { "No tests yet".to_string() } else { format!("{n} tests in your list") }.into());
    }
    fn save_settings(&self) {
        let ui = self.ui();
        if ui.get_s_name().trim().is_empty() { ui.set_err_s_name("Please enter your lab's name".into()); self.toast("Please enter your lab's name first", true); return; }
        ui.set_err_s_name("".into());
        let num = |s: SharedString, label: &str| -> Result<f32, String> { s.trim().parse::<f32>().map_err(|_| format!("{label} space must be a number of millimetres, for example 40")) };
        let (top, bottom, left, right) = match (num(ui.get_s_top(), "Top"), num(ui.get_s_bottom(), "Bottom"), num(ui.get_s_left(), "Left"), num(ui.get_s_right(), "Right")) {
            (Ok(a), Ok(b), Ok(c), Ok(d)) => (a, b, c, d),
            (Err(m), _, _, _) | (_, Err(m), _, _) | (_, _, Err(m), _) | (_, _, _, Err(m)) => return self.toast(&m, true),
        };
        let existing = match self.st.borrow().store.profile() { Ok(p) => p, Err(e) => return self.fail(e) };
        let p = Profile { name: ui.get_s_name().into(), address: ui.get_s_address().into(), phone: ui.get_s_phone().into(), email: ui.get_s_email().into(),
            signatory: ui.get_s_signatory().into(), qualification: ui.get_s_qualification().into(), top_mm: top, bottom_mm: bottom, left_mm: left, right_mm: right, text_size: [80u8, 90, 100, 110][ui.get_s_size().clamp(0, 3) as usize], ..existing };
        let actor = self.actor();
        let result = self.st.borrow_mut().store.save_profile(&actor, &p);
        match result { Ok(()) => { self.toast("Saved", false); self.refresh_home(); } Err(e) => self.fail(e) }
    }
    fn choose_image(&self, kind: &str) {
        let title = if kind == "letterhead" { "Choose your letterhead (PNG or JPG)" } else { "Choose the signature image (PNG or JPG)" };
        let Some(path) = rfd::FileDialog::new().set_title(title).add_filter("Pictures", &["png", "jpg", "jpeg"]).pick_file() else { return };
        self.set_image(kind, &path);
    }
    pub fn set_image(&self, kind: &str, path: &Path) {
        let bytes = match std::fs::read(path) { Ok(b) => b, Err(e) => return self.toast(&format!("Could not open that file: {e}"), true) };
        let actor = self.actor();
        let result = self.st.borrow_mut().store.set_asset(&actor, kind, bytes);
        match result {
            Ok(()) => { self.load_settings(); self.refresh_home(); self.toast(if kind == "letterhead" { "Letterhead added. Press PRINT A TEST PAGE to see how it lines up." } else { "Signature added" }, false); }
            Err(e) => self.fail(e),
        }
    }
    fn remove_image(&self, kind: &str) {
        let actor = self.actor();
        let result = self.st.borrow_mut().store.clear_asset(&actor, kind);
        match result { Ok(()) => { self.ui().set_has_letterhead(self.st.borrow().store.asset("letterhead").ok().flatten().is_some()); self.load_settings(); self.refresh_home(); self.toast("Removed", false); } Err(e) => self.fail(e) }
    }
    fn import_tests(&self) {
        let Some(path) = rfd::FileDialog::new().set_title("Choose your test list (.json)").add_filter("Test list", &["json"]).pick_file() else { return };
        match std::fs::read_to_string(&path) { Ok(text) => self.import_text(&text), Err(e) => self.toast(&format!("Could not open that file: {e}"), true) }
    }
    fn load_samples(&self) { self.import_text(STANDARD_TESTS); }
    fn import_text(&self, text: &str) {
        let actor = self.actor();
        let result = self.st.borrow_mut().store.import_catalogue(&actor, text);
        match result {
            Ok(n) => { self.refresh_tests(); self.refresh_admin_tests(); self.load_settings(); self.refresh_home(); self.toast(&format!("{n} tests are in your list now. Prices are not set yet: open a test to type its price."), false); }
            Err(e) => self.fail(e),
        }
    }
}

// ---------- signing in ----------
impl Ctl {
    fn enter(&self, user: User) { self.enter_as(user, false); }
    fn enter_as(&self, user: User, resumed: bool) {
        let ui = self.ui();
        ui.set_user_name(user.name.as_str().into());
        ui.set_user_role(user.role.label().into());
        ui.set_is_admin(user.role == Role::Admin); ui.set_can_money(matches!(user.role, Role::Admin | Role::Reception));
        ui.set_s_tab(if user.role == Role::Admin { 0 } else { 5 });
        ui.set_page(0);
        for f in [AppWindow::set_l_pass, AppWindow::set_l_user, AppWindow::set_a_pass, AppWindow::set_a_pass2, AppWindow::set_a_user, AppWindow::set_a_name] { f(&ui, "".into()); }
        for f in [AppWindow::set_err_login, AppWindow::set_err_a_name, AppWindow::set_err_a_user, AppWindow::set_err_a_pass, AppWindow::set_err_a_pass2] { f(&ui, "".into()); }
        let first_name = user.name.split_whitespace().next().unwrap_or("").to_string();
        self.st.borrow_mut().user = Some(user);
        ui.set_signed_in(true);
        ui.set_auth_first_run(false);
        self.refresh_tests(); self.refresh_orders(); self.refresh_bill(); self.refresh_home(); self.load_settings(); self.refresh_admin_tests(); self.refresh_users();
        self.default_backup_folder();
        self.refresh_data();
        if !resumed { self.toast(&format!("Welcome, {first_name}"), false); }
        self.auto_backup();
    }
    /// Pick up where the person left off: the saved sign-in from the last run, if it is still valid.
    fn resume_saved_sign_in(&self) {
        let Some(f) = self.session_file() else { return };
        let Ok(token) = std::fs::read_to_string(&f) else { return };
        let r = self.st.borrow_mut().store.resume_session(&token);
        match r {
            Ok(Some(u)) => self.enter_as(u, true),
            _ => { let _ = std::fs::remove_file(&f); }
        }
    }
    fn session_file(&self) -> Option<PathBuf> { self.opts.use_sessions.then(|| self.opts.data_dir.join("session.token")) }
    /// Keep (or forget) this sign-in for the next time the program is opened.
    fn remember(&self, user_id: i64, keep: bool) {
        let Some(f) = self.session_file() else { return };
        if let Ok(old) = std::fs::read_to_string(&f) { let _ = self.st.borrow_mut().store.end_session(&old); }
        if !keep { let _ = std::fs::remove_file(&f); return; }
        let token = self.st.borrow_mut().store.create_session(user_id);
        match token {
            Ok(t) => { if std::fs::write(&f, t).is_err() { self.toast("Could not remember this sign-in on this computer. You will have to sign in next time.", true); } }
            Err(e) => self.fail(e),
        }
    }
    fn forget_session(&self) {
        let Some(f) = self.session_file() else { return };
        if let Ok(old) = std::fs::read_to_string(&f) { let _ = self.st.borrow_mut().store.end_session(&old); }
        let _ = std::fs::remove_file(&f);
    }

    fn sign_in(&self) {
        let ui = self.ui();
        let (u, p) = (ui.get_l_user().to_string(), ui.get_l_pass().to_string());
        if u.trim().is_empty() || p.is_empty() { ui.set_err_login("Please type your username and password".into()); return; }
        let result = self.st.borrow_mut().store.authenticate(&u, &p);
        match result {
            Ok(user) => { self.remember(user.id, ui.get_l_keep()); self.enter(user); }
            Err(Error::Locked(n)) => { ui.set_l_pass("".into()); self.start_lock_countdown(n); }
            Err(e) => {
                ui.set_l_pass("".into());
                let left = self.st.borrow().store.attempts_left().unwrap_or(5);
                let hint = if left > 0 && left <= 2 { format!(" {left} {} left before a 1-minute wait.", if left == 1 { "try is" } else { "tries are" }) } else { String::new() };
                ui.set_err_login(format!("{}{hint}", friendly(&e)).into());
            }
        }
    }

    /// Show "please wait N seconds" and count down on its own. Works after the program was closed and reopened too.
    fn start_lock_countdown(&self, secs: u64) {
        let ui = self.ui();
        ui.set_err_login(lock_text(secs).into());
        if ui.get_auth_forgot() { ui.set_err_f_code(lock_text(secs).into()); }
        let me = self.me.borrow().clone();
        self.lock_timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
            let Some(c) = me.upgrade() else { return };
            let ui = c.ui();
            let left = c.st.borrow().store.lock_seconds_left().unwrap_or(0);
            if left == 0 {
                c.lock_timer.stop();
                if ui.get_err_login().starts_with("Too many") { ui.set_err_login("".into()); }
                if ui.get_err_f_code().starts_with("Too many") { ui.set_err_f_code("".into()); }
            } else {
                if ui.get_err_login().starts_with("Too many") { ui.set_err_login(lock_text(left).into()); }
                if ui.get_err_f_code().starts_with("Too many") { ui.set_err_f_code(lock_text(left).into()); }
            }
        });
    }
    fn show_lock_if_any(&self) {
        let left = self.st.borrow().store.lock_seconds_left().unwrap_or(0);
        if left > 0 { self.start_lock_countdown(left); }
    }

    fn create_first_admin(&self) {
        let ui = self.ui();
        let (name, user, p1, p2) = (ui.get_a_name().to_string(), ui.get_a_user().to_string(), ui.get_a_pass().to_string(), ui.get_a_pass2().to_string());
        if p1 != p2 { ui.set_err_a_pass2("The two passwords are not the same".into()); return; }
        let result = self.st.borrow_mut().store.create_first_admin(&user, &name, &p1);
        match result {
            Ok(u) => {
                // a new lab starts with the standard test list, so it can register patients straight away
                let empty = self.st.borrow().store.tests().map(|t| t.is_empty()).unwrap_or(false);
                if empty { let _ = self.st.borrow_mut().store.import_catalogue(&u.actor(), STANDARD_TESTS); }
                let actor = u.actor();
                self.remember(u.id, ui.get_l_keep());
                self.enter(u);
                let code = self.st.borrow_mut().store.new_recovery_code(&actor);
                match code { Ok(c) => self.show_code(&c), Err(e) => self.fail(e) }
            }
            Err(e) => {
                let m = friendly(&e);
                let l = m.to_lowercase();
                if l.contains("username") { ui.set_err_a_user(m.into()); } else if l.contains("password") { ui.set_err_a_pass(m.into()); } else { ui.set_err_a_name(m.into()); }
            }
        }
    }

    fn sign_out(&self) {
        let ui = self.ui();
        self.forget_session();
        ui.set_edit_shown(false);
        {
            let mut st = self.st.borrow_mut();
            st.user = None; st.order = None; st.entries.clear(); st.selected.clear(); st.te_base = None; st.tp.clear(); st.sel.clear();
            st.params.set_vec(vec![]); st.row_model.set_vec(vec![]); st.attention.set_vec(vec![]); st.orders.set_vec(vec![]);
        }
        ui.set_te_id(-1); ui.set_ue_id(-1); ui.set_page(0); ui.set_is_admin(false); ui.set_can_money(false);
        self.clear_link(); ui.set_pat_selected(0); ui.set_pat_query("".into());
        for f in [AppWindow::set_f_first, AppWindow::set_f_last, AppWindow::set_f_age, AppWindow::set_f_phone, AppWindow::set_f_address, AppWindow::set_f_discount, AppWindow::set_f_paid, AppWindow::set_ma_old, AppWindow::set_ma_new, AppWindow::set_ma_new2, AppWindow::set_ue_pass, AppWindow::set_f_email, AppWindow::set_f_doctor, AppWindow::set_bk_password, AppWindow::set_help_query] { f(&ui, "".into()); }
        self.hide_toast();
        self.confirm_close();
        ui.set_auth_forgot(false); ui.set_rc_shown(false);
        ui.set_signed_in(false);
    }

    fn change_my_password(&self) {
        let ui = self.ui();
        let Some(id) = self.st.borrow().user.as_ref().map(|u| u.id) else { return };
        let (old, new, again) = (ui.get_ma_old().to_string(), ui.get_ma_new().to_string(), ui.get_ma_new2().to_string());
        ui.set_err_ma_old("".into()); ui.set_err_ma_new("".into());
        if new != again { ui.set_err_ma_new("The two new passwords are not the same".into()); return; }
        let had_session = self.session_file().is_some_and(|f| f.exists());
        let result = self.st.borrow_mut().store.change_password(id, &old, &new);
        match result {
            Ok(()) => { if had_session { self.remember(id, true); } ui.set_ma_old("".into()); ui.set_ma_new("".into()); ui.set_ma_new2("".into()); self.toast("Your password was changed", false); }
            Err(e) => { let m = friendly(&e); if m.contains("current") { ui.set_err_ma_old(m.into()); } else { ui.set_err_ma_new(m.into()); } }
        }
    }
}

// ---------- password recovery ----------
impl Ctl {
    fn set_forgot(&self, on: bool) {
        let ui = self.ui();
        for f in [AppWindow::set_err_f_user, AppWindow::set_err_f_code, AppWindow::set_err_f_pass, AppWindow::set_err_f_pass2, AppWindow::set_err_login] { f(&ui, "".into()); }
        ui.set_f_user(ui.get_l_user()); ui.set_f_found("".into()); ui.set_auth_forgot(on);
    }
    fn show_code(&self, code: &str) { let ui = self.ui(); ui.set_rc_code(code.into()); ui.set_rc_shown(true); }
    fn ask_new_recovery(&self) {
        self.ask(Pending::NewRecovery, "Make a new recovery code?", "The old recovery code will stop working. You will see the new one once, so have a pen or a USB drive ready.\n\nPlease type your own password to confirm.", "YES, MAKE A NEW CODE", true);
    }
    fn show_new_recovery(&self) {
        let actor = self.actor();
        let r = self.st.borrow_mut().store.new_recovery_code(&actor);
        match r { Ok(c) => { self.load_settings(); self.show_code(&c); } Err(e) => self.fail(e) }
    }
    fn rc_save_copy(&self) {
        let ui = self.ui();
        let code = ui.get_rc_code().to_string();
        let lab = self.st.borrow().store.profile().map(|p| p.name).unwrap_or_default();
        let Some(path) = rfd::FileDialog::new().set_title("Save the recovery code (best on a USB drive)").set_file_name("LabDesk-recovery-code.txt").save_file() else { return };
        let text = format!("LabDesk recovery code{}\r\n\r\n{code}\r\n\r\nKeep this paper or file away from the lab computer.\r\nIf an Admin forgets their password: on the sign-in screen press \"Forgot your password?\", type the Admin username, this code and a new password.\r\nAnyone who has this code can take over an Admin account.\r\n", if lab.is_empty() { String::new() } else { format!(" for {lab}") });
        match std::fs::write(&path, text) { Ok(()) => self.toast("A copy of the code was saved", false), Err(e) => self.toast(&format!("Could not save the file: {e}"), true) }
    }
    /// The recovery code alone shows the Admin username, and fills it in for the next step.
    fn find_username(&self) {
        let ui = self.ui();
        for f in [AppWindow::set_err_f_user, AppWindow::set_err_f_code] { f(&ui, "".into()); }
        ui.set_f_found("".into());
        let code = ui.get_f_code().to_string();
        if code.trim().is_empty() { ui.set_err_f_code("Type the recovery code first".into()); return; }
        let r = self.st.borrow_mut().store.admin_usernames(&code);
        match r {
            Ok(names) if names.is_empty() => ui.set_err_f_code("No Admin account was found".into()),
            Ok(names) => { ui.set_f_found(names.join(", ").into()); ui.set_f_user(names[0].clone().into()); }
            Err(Error::Locked(n)) => { ui.set_f_code("".into()); self.start_lock_countdown(n); }
            Err(e) => { ui.set_f_code("".into()); ui.set_err_f_code(friendly(&e).into()); }
        }
    }
    fn recover(&self) {
        let ui = self.ui();
        for f in [AppWindow::set_err_f_user, AppWindow::set_err_f_code, AppWindow::set_err_f_pass, AppWindow::set_err_f_pass2] { f(&ui, "".into()); }
        let (user, code, p1, p2) = (ui.get_f_user().to_string(), ui.get_f_code().to_string(), ui.get_f_pass().to_string(), ui.get_f_pass2().to_string());
        let mut bad = false;
        if user.trim().is_empty() { ui.set_err_f_user("Please type the Admin username".into()); bad = true; }
        if code.trim().is_empty() { ui.set_err_f_code("Please type the recovery code".into()); bad = true; }
        if let Err(e) = labdesk_core::users::check_password(&p1) { ui.set_err_f_pass(friendly(&e).into()); bad = true; }
        else if p1 != p2 { ui.set_err_f_pass2("The two passwords are not the same".into()); bad = true; }
        if bad { return; }
        let result = self.st.borrow_mut().store.recover_admin(&user, &code, &p1);
        match result {
            Ok((u, fresh)) => {
                self.remember(u.id, ui.get_l_keep());
                for f in [AppWindow::set_f_code, AppWindow::set_f_pass, AppWindow::set_f_pass2, AppWindow::set_f_user] { f(&ui, "".into()); }
                ui.set_auth_forgot(false);
                self.enter(u);
                self.show_code(&fresh);
                self.toast("Your new password is set. You are signed in.", false);
            }
            Err(Error::Locked(n)) => { ui.set_f_code("".into()); self.start_lock_countdown(n); }
            Err(e) => {
                ui.set_f_code("".into());
                ui.set_err_f_code(friendly(&e).into());
            }
        }
    }
}

// ---------- people ----------
impl Ctl {
    pub fn refresh_users(&self) {
        let ui = self.ui();
        let st = self.st.borrow();
        if !st.user.as_ref().is_some_and(|u| u.role == Role::Admin) { return; }
        let list = st.store.users().unwrap_or_default();
        st.users.set_vec(list.iter().map(|u| UserRow { id: u.id as i32, name: u.name.as_str().into(), username: u.username.as_str().into(), role: u.role.label().into(), active: u.active }).collect::<Vec<_>>());
        let _ = ui;
    }
    fn ue_clear_errors(&self) { let ui = self.ui(); ui.set_err_ue_name("".into()); ui.set_err_ue_user("".into()); ui.set_err_ue_pass("".into()); ui.set_ue_pass("".into()); }
    fn my_id(&self) -> i64 { self.st.borrow().user.as_ref().map_or(-1, |u| u.id) }
    fn ue_new(&self) {
        let ui = self.ui();
        self.ue_clear_errors();
        ui.set_ue_id(0); ui.set_ue_name("".into()); ui.set_ue_user("".into()); ui.set_ue_role(0); ui.set_ue_state(0);
        ui.set_ue_lock_demote(false); ui.set_ue_target_admin(false); ui.set_ue_can_remove(false);
    }
    fn ue_open(&self, id: i64) {
        let ui = self.ui();
        let list = self.st.borrow().store.users().unwrap_or_default();
        let Some(u) = list.iter().find(|u| u.id == id).cloned() else { return };
        self.ue_clear_errors();
        ui.set_ue_id(id as i32); ui.set_ue_name(u.name.as_str().into()); ui.set_ue_user(u.username.as_str().into());
        ui.set_ue_role(ROLE_ORDER.iter().position(|r| *r == u.role).unwrap_or(0) as i32); ui.set_ue_state(if u.active { 0 } else { 1 });
        let other_admins = list.iter().any(|o| o.id != id && o.role == Role::Admin && o.active);
        ui.set_ue_lock_demote(u.role == Role::Admin && u.active && !other_admins);
        ui.set_ue_target_admin(u.role == Role::Admin);
        ui.set_ue_can_remove(id != self.my_id() && !(u.role == Role::Admin && u.active && !other_admins));
        self.refresh_users();
    }

    /// Check the form, then ask "Are you sure?" before anything is changed.
    fn ue_save(&self) {
        let ui = self.ui();
        let id = ui.get_ue_id() as i64;
        let (name, user, pass) = (ui.get_ue_name().to_string(), ui.get_ue_user().to_string(), ui.get_ue_pass().to_string());
        self.ue_clear_errors_keep_pass();
        let mut bad = false;
        if name.trim().is_empty() { ui.set_err_ue_name("Please enter the person's name".into()); bad = true; }
        if id == 0 {
            if let Err(e) = labdesk_core::users::clean_username(&user) { ui.set_err_ue_user(friendly(&e).into()); bad = true; }
            if let Err(e) = labdesk_core::users::check_password(&pass) { ui.set_err_ue_pass(friendly(&e).into()); bad = true; }
        } else if !pass.is_empty() {
            if let Err(e) = labdesk_core::users::check_password(&pass) { ui.set_err_ue_pass(friendly(&e).into()); bad = true; }
        }
        if bad { return self.toast("Please check the highlighted fields", true); }
        let role = ROLE_ORDER[(ui.get_ue_role().clamp(0, 3)) as usize];
        if id == 0 {
            self.ask(Pending::CreateUser, &format!("Add {}?", name.trim()), &format!("{} will be able to sign in with the username \"{}\" as {}.\n\nTo make sure it is really you, please type your own password.", name.trim(), user.trim().to_lowercase(), role.label()), "YES, ADD THIS PERSON", true);
        } else if !pass.is_empty() {
            self.ask(Pending::SaveUser, "Save these changes?", &format!("The changes to {} will be saved and their password will be changed.\n\nPlease type your own password to confirm.", name.trim()), "YES, SAVE", true);
        } else {
            self.ask(Pending::SaveUser, "Save these changes?", &format!("Are you sure you want to save the changes to {}?", name.trim()), "YES, SAVE", false);
        }
    }
    fn ue_clear_errors_keep_pass(&self) { let ui = self.ui(); ui.set_err_ue_name("".into()); ui.set_err_ue_user("".into()); ui.set_err_ue_pass("".into()); }
    fn ue_remove(&self) {
        let ui = self.ui();
        let id = ui.get_ue_id() as i64;
        let name = ui.get_ue_name().to_string();
        self.ask(Pending::RemoveUser(id), &format!("Remove {}?", name), &format!("{} will no longer be able to sign in, and the account is deleted. This cannot be undone.\n\nPlease type your own password to confirm.", name), "YES, REMOVE", true);
    }

    // ----- the "Are you sure?" box -----
    fn ask(&self, what: Pending, title: &str, text: &str, yes: &str, needs_password: bool) {
        let ui = self.ui();
        self.st.borrow_mut().pending = Some(what);
        ui.set_confirm_no_text("NO, GO BACK".into());
        ui.set_confirm_title(title.into()); ui.set_confirm_text(text.into()); ui.set_confirm_yes(yes.into());
        ui.set_confirm_needs_password(needs_password); ui.set_confirm_pass("".into()); ui.set_err_confirm("".into());
        ui.set_confirm_extra_label("".into()); ui.set_confirm_extra("".into()); ui.set_err_confirm_extra("".into()); ui.set_confirm_extra_secret(false);
        ui.set_confirm_shown(true);
    }
    /// "No" in the box. For "is this the same person?" it means: this is somebody new, go ahead and register.
    fn confirm_no(&self) {
        let was = self.st.borrow().pending;
        self.confirm_close();
        if matches!(was, Some(Pending::SamePerson(_))) { self.st.borrow_mut().skip_match = true; self.register(); }
    }
    fn confirm_close(&self) {
        self.st.borrow_mut().pending = None;
        let ui = self.ui();
        ui.set_confirm_shown(false); ui.set_confirm_pass("".into()); ui.set_err_confirm("".into()); ui.set_confirm_extra("".into()); ui.set_err_confirm_extra("".into());
    }
    fn confirm_yes(&self) {
        let ui = self.ui();
        let Some(what) = self.st.borrow().pending else { return self.confirm_close() };
        let extra = ui.get_confirm_extra().to_string();
        if matches!(what, Pending::DeleteOrder(_)) && extra.trim().chars().count() < 3 { ui.set_err_confirm_extra("Please write a short reason, for example: entered twice".into()); return; }
        if ui.get_confirm_extra_secret() && extra.is_empty() { ui.set_err_confirm_extra("Please type the backup password".into()); return; }
        if ui.get_confirm_needs_password() {
            let pw = ui.get_confirm_pass().to_string();
            if pw.is_empty() { ui.set_err_confirm("Please type your password".into()); return; }
            let my = self.my_id();
            let ok = self.st.borrow_mut().store.confirm_password(my, &pw);
            if let Err(e) = ok { ui.set_confirm_pass("".into()); ui.set_err_confirm(friendly(&e).into()); return; }
        }
        self.confirm_close();
        match what {
            Pending::CreateUser | Pending::SaveUser => self.ue_apply(what == Pending::CreateUser),
            Pending::RemoveUser(id) => self.ue_delete(id),
            Pending::NewRecovery => self.show_new_recovery(),
            Pending::DeleteOrder(id) => self.delete_order(id, extra.trim()),
            Pending::SamePerson(id) => { self.pick_patient(id); self.toast("Using the saved details of this patient. Please check them, then press Register.", false); }
            Pending::SendEmail(id) => self.do_send_email(id),
            Pending::Restore => self.do_restore(false, &extra),
            Pending::RestoreFirstRun => self.do_restore(true, &extra),
        }
    }
    fn ue_delete(&self, id: i64) {
        let actor = self.actor();
        let my = self.my_id();
        let result = self.st.borrow_mut().store.delete_user(&actor, my, id);
        match result {
            Ok(()) => { let ui = self.ui(); ui.set_ue_id(-1); self.refresh_users(); self.toast("The account was removed", false); }
            Err(e) => self.fail(e),
        }
    }
    /// Really create or change the account (called after the person said yes).
    fn ue_apply(&self, creating: bool) {
        let ui = self.ui();
        let id = ui.get_ue_id() as i64;
        let role = ROLE_ORDER[(ui.get_ue_role().clamp(0, 3)) as usize];
        let (name, user, pass) = (ui.get_ue_name().to_string(), ui.get_ue_user().to_string(), ui.get_ue_pass().to_string());
        let actor = self.actor();
        let result: Result<String, Error> = if creating {
            self.st.borrow_mut().store.create_user(&actor, &user, &name, role, &pass).map(|u| format!("{} can now sign in with the username \"{}\"", u.name, u.username))
        } else {
            (|| {
                let mut st = self.st.borrow_mut();
                st.store.update_user(&actor, id, &name, role, ui.get_ue_state() == 0)?;
                if !pass.is_empty() { st.store.reset_password(&actor, id, &pass)?; }
                Ok(if pass.is_empty() { "Saved".to_string() } else { "Saved, with the new password".to_string() })
            })()
        };
        match result {
            Ok(msg) => {
                if self.st.borrow().user.as_ref().map(|u| u.id) == Some(id) { self.refresh_session_user(); }
                self.refresh_users();
                let new_id = if creating { self.st.borrow().store.users().ok().and_then(|l| l.into_iter().find(|u| u.username == user.trim().to_lowercase())).map(|u| u.id) } else { Some(id) };
                if let Some(n) = new_id { self.ue_open(n); }
                self.toast(&msg, false);
            }
            Err(e) => {
                let m = friendly(&e); let l = m.to_lowercase();
                if l.contains("username") { ui.set_err_ue_user(m.into()); }
                else if l.contains("password") && !l.contains("admin") { ui.set_err_ue_pass(m.into()); }
                else { self.toast(&m, true); }
            }
        }
    }
    /// Re-read the signed-in person's own record (name or role may have just changed).
    fn refresh_session_user(&self) {
        let id = self.st.borrow().user.as_ref().map(|u| u.id);
        let fresh = id.and_then(|id| self.st.borrow().store.users().ok().and_then(|l| l.into_iter().find(|u| u.id == id)));
        if let Some(u) = fresh { let ui = self.ui(); ui.set_is_admin(u.role == Role::Admin); ui.set_can_money(matches!(u.role, Role::Admin | Role::Reception)); ui.set_user_name(u.name.as_str().into()); ui.set_user_role(u.role.label().into()); self.st.borrow_mut().user = Some(u); self.refresh_header(); }
    }
}

// ---------- test editor ----------
struct SumCounts { tests: usize, results: usize, abnormal: usize, critical: usize, blank: usize }
/// The tests of a report as readable rows: a header per test, then each result line that has a value.
/// `only_open` leaves out tests that were already approved (used for the approve summary).
fn sum_rows(o: &Order, only_open: bool) -> (Vec<SumRow>, SumCounts) {
    let mut rows = Vec::new();
    let mut n = SumCounts { tests: 0, results: 0, abnormal: 0, critical: 0, blank: 0 };
    for t in o.tests.iter().filter(|t| !only_open || !t.is_approved()) {
        n.tests += 1;
        rows.push(SumRow { kind: 0, name: t.name.as_str().into(), value: "".into(), range: "".into(), flag: "".into() });
        for p in t.config.parameters.iter().filter(|p| !p.hidden && p.is_result_field()) {
            let r = t.results.iter().find(|r| r.parameter_id == p.id).filter(|r| !r.value.trim().is_empty());
            let Some(r) = r else { n.blank += 1; continue };
            n.results += 1;
            if !r.flag.is_empty() { n.abnormal += 1; }
            if r.flag == "Critical" { n.critical += 1; }
            let value = if p.unit.is_empty() { r.value.clone() } else { format!("{} {}", r.value, p.unit) };
            rows.push(SumRow { kind: 1, name: p.name.as_str().into(), value: value.into(), range: result_range(p, r.range_text.as_deref(), &o.patient).into(), flag: r.flag.as_str().into() });
        }
    }
    (rows, n)
}
fn num_text(v: Option<f64>) -> SharedString { v.map(|x| format!("{x}")).unwrap_or_default().into() }

/// What the Range column of the fields table shows.
fn field_range(p: &Parameter) -> String {
    if !p.is_result_field() { return String::new(); }
    if !p.range_text.trim().is_empty() { return labdesk_core::calc::strip_gender_label(&p.range_text); }
    let n = |v: f64| format!("{v}");
    let base = match (p.low, p.high) { (Some(l), Some(h)) => format!("{}-{}", n(l), n(h)), (Some(l), None) => format!(">= {}", n(l)), (None, Some(h)) => format!("<= {}", n(h)), _ => String::new() };
    if base.is_empty() && (!p.ranges.is_empty() || !p.gender_ranges.is_empty()) { "by age / gender".into() } else { base }
}
fn blank_arow() -> ARow { ARow { gender: "Both".into(), from: "".into(), to: "".into(), unit: "Years".into(), low: "".into(), high: "".into(), clow: "".into(), chigh: "".into(), text: "".into() } }
fn overlay_row(o: &labdesk_core::RangeOverlay) -> ARow {
    let bounds: Vec<f64> = [o.age_min_days, o.age_max_days].into_iter().flatten().collect();
    let near = |v: f64, k: f64| ((v / k) - (v / k).round()).abs() < 0.02;
    let (unit, k) = if bounds.iter().all(|v| near(*v, 365.25)) { ("Years", 365.25) } else if bounds.iter().all(|v| near(*v, 30.4375)) { ("Months", 30.4375) } else { ("Days", 1.0) };
    let t = |v: Option<f64>| -> SharedString { v.map(|x| format!("{}", (x * 100.0).round() / 100.0)).unwrap_or_default().into() };
    ARow { gender: o.gender.clone().filter(|g| g == "Male" || g == "Female").unwrap_or_else(|| "Both".into()).into(), from: t(o.age_min_days.map(|v| v / k)), to: t(o.age_max_days.map(|v| v / k)), unit: unit.into(),
        low: num_text(o.low), high: num_text(o.high), clow: num_text(o.critical_low), chigh: num_text(o.critical_high), text: o.range_text.clone().unwrap_or_default().into() }
}
fn parse_num(label: &str, line: &str, t: &str) -> Result<Option<f64>, String> {
    let t = t.trim();
    if t.is_empty() { return Ok(None); }
    t.parse::<f64>().ok().filter(|v| v.is_finite()).map(Some).ok_or_else(|| format!("{label} of \"{line}\" must be a number, for example 13 or 4.5 (you typed \"{t}\")"))
}
impl Ctl {
    pub fn refresh_admin_tests(&self) {
        let ui = self.ui();
        let q = ui.get_te_query().to_string().to_lowercase();
        let st = self.st.borrow();
        if !st.user.as_ref().is_some_and(|u| u.role == Role::Admin) { return; }
        let all = st.store.tests().unwrap_or_default();
        ui.set_test_count_text(if all.is_empty() { "No tests yet".to_string() } else { format!("{} tests", all.len()) }.into());
        st.admin_tests.set_vec(all.iter().filter(|t| q.is_empty() || t.name.to_lowercase().contains(&q) || t.code.to_lowercase().contains(&q) || t.department.to_lowercase().contains(&q))
            .map(|t| TestAdminRow { id: t.id as i32, code: t.code.as_str().into(), name: t.name.as_str().into(), dept: t.department.as_str().into(), price: fmt::rupees(t.price_paise).into(), off: t.disabled }).collect::<Vec<_>>());
    }
    fn te_errors_off(&self) { let ui = self.ui(); ui.set_err_te_code("".into()); ui.set_err_te_name("".into()); ui.set_err_te_dept("".into()); ui.set_err_te_price("".into()); ui.set_err_fp_name("".into()); }

    // ----- the list of fields shown in the table -----
    fn show_rows(&self) {
        let ui = self.ui();
        let fp = ui.get_fp_idx();
        let st = self.st.borrow();
        st.row_model.set_vec(st.tp.iter().enumerate().map(|(i, p)| FRow {
            name: p.name.as_str().into(), field: if p.field == "Title" { "Heading".into() } else { p.field.as_str().into() }, unit: p.unit.as_str().into(),
            range: field_range(p).into(), formula: !p.formula.is_empty(), selected: st.sel.get(i).copied().unwrap_or(false), active: fp == i as i32,
            title: p.field == "Title", hidden: p.hidden,
        }).collect::<Vec<_>>());
    }
    fn fp_clear(&self) {
        let ui = self.ui();
        ui.set_fp_idx(-1); ui.set_fp_kind(0); ui.set_fp_name("".into()); ui.set_fp_method("".into()); ui.set_fp_code("".into()); ui.set_fp_field("Numeric".into()); ui.set_fp_unit("".into());
        ui.set_fp_from("".into()); ui.set_fp_to("".into()); ui.set_fp_crit(false); ui.set_fp_clow("".into()); ui.set_fp_chigh("".into()); ui.set_fp_rtext("".into()); ui.set_fp_formula("".into());
        ui.set_fp_options("".into()); ui.set_fp_required(false); ui.set_fp_hidden(false); ui.set_fp_omit(false); ui.set_fp_range_count("".into()); ui.set_err_fp_name("".into());
        self.st.borrow_mut().fp_ranges = None;
        self.show_rows();
    }
    fn fp_load(&self, i: i32) {
        let ui = self.ui();
        let Some(p) = self.st.borrow().tp.get(i as usize).cloned() else { return };
        ui.set_fp_idx(i); ui.set_fp_kind((p.field == "Title") as i32); ui.set_fp_name(p.name.as_str().into()); ui.set_fp_method(p.method.as_str().into()); ui.set_fp_code(p.code.as_str().into());
        ui.set_fp_field(if matches!(p.field.as_str(), "Numeric" | "Text" | "Dropdown" | "Multiple ranges") { p.field.as_str() } else { "Numeric" }.into());
        ui.set_fp_unit(p.unit.as_str().into()); ui.set_fp_from(num_text(p.low)); ui.set_fp_to(num_text(p.high));
        ui.set_fp_crit(p.critical_low.is_some() || p.critical_high.is_some()); ui.set_fp_clow(num_text(p.critical_low)); ui.set_fp_chigh(num_text(p.critical_high));
        ui.set_fp_rtext(p.range_text.as_str().into()); ui.set_fp_formula(p.formula.as_str().into()); ui.set_fp_options(p.options.join(", ").into());
        ui.set_fp_required(p.required); ui.set_fp_hidden(p.hidden); ui.set_fp_omit(p.omit_dlc); ui.set_err_fp_name("".into());
        let n = p.ranges.len() + p.gender_ranges.len();
        ui.set_fp_range_count(if n == 0 { String::new() } else { format!("{n} age / gender range(s) set. Press VIEW AGE RANGES to see or change them.") }.into());
        self.st.borrow_mut().fp_ranges = None;
        self.show_rows();
    }
    fn fp_submit(&self) {
        let ui = self.ui();
        ui.set_err_fp_name("".into());
        let name = ui.get_fp_name().trim().to_string();
        if name.is_empty() { ui.set_err_fp_name("Please enter a name".into()); return self.toast("Every field needs a name", true); }
        let idx = ui.get_fp_idx();
        let mut p = if idx >= 0 { self.st.borrow().tp.get(idx as usize).cloned().unwrap_or_default() } else { Parameter { field: "Numeric".into(), ..Default::default() } };
        let old = (p.low, p.high, p.range_text.clone());
        p.name = name.clone();
        if ui.get_fp_kind() == 1 {
            p.field = "Title".into(); p.unit.clear(); p.low = None; p.high = None; p.critical_low = None; p.critical_high = None; p.range_text.clear(); p.formula.clear(); p.options.clear();
            p.method.clear(); p.code.clear(); p.required = false; p.hidden = ui.get_fp_hidden(); p.omit_dlc = false;
        } else {
            let field = ui.get_fp_field().to_string();
            let nums = (|| -> Result<_, String> { Ok((parse_num("The range start", &name, &ui.get_fp_from())?, parse_num("The range end", &name, &ui.get_fp_to())?,
                parse_num("The critical \"less than\"", &name, &ui.get_fp_clow())?, parse_num("The critical \"more than\"", &name, &ui.get_fp_chigh())?)) })();
            let (lo, hi, cl, ch) = match nums { Ok(v) => v, Err(m) => return self.toast(&m, true) };
            let numeric = matches!(field.as_str(), "Numeric" | "Multiple ranges");
            if numeric && lo.zip(hi).is_some_and(|(l, h)| l > h) { return self.toast("The range starts above where it ends. Please swap the two numbers.", true); }
            p.field = field.clone();
            p.method = ui.get_fp_method().trim().to_string(); p.code = ui.get_fp_code().trim().to_string(); p.unit = ui.get_fp_unit().trim().to_string();
            p.options = if field == "Dropdown" { ui.get_fp_options().split(',').map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect() } else { vec![] };
            if field == "Dropdown" && p.options.is_empty() { return self.toast("Type the choices for this dropdown, separated by commas, for example: Pale yellow, Yellow, Amber", true); }
            if numeric {
                p.low = lo; p.high = hi;
                let crit = ui.get_fp_crit();
                p.critical_low = if crit { cl } else { None }; p.critical_high = if crit { ch } else { None };
            } else { p.low = None; p.high = None; p.critical_low = None; p.critical_high = None; }
            let mut rt = ui.get_fp_rtext().trim().to_string();
            // an old printed range that was only a copy of the previous numbers must not hide the new numbers
            if (p.low, p.high) != (old.0, old.1) && rt == old.2 { rt.clear(); }
            p.range_text = rt;
            p.formula = ui.get_fp_formula().trim().to_string();
            p.required = ui.get_fp_required(); p.hidden = ui.get_fp_hidden(); p.omit_dlc = ui.get_fp_omit();
            if let Some(r) = self.st.borrow().fp_ranges.clone() { p.ranges = r; p.gender_ranges.clear(); }
        }
        {
            let mut st = self.st.borrow_mut();
            if idx >= 0 && (idx as usize) < st.tp.len() { st.tp[idx as usize] = p; } else { st.tp.push(p); st.sel.push(false); }
        }
        let was_new = idx < 0;
        self.fp_clear();
        self.toast(if was_new { "Field added. Press SAVE (top right) to keep the test." } else { "Field changed. Press SAVE (top right) to keep the test." }, false);
    }
    fn row_delete(&self, i: usize) {
        {
            let mut st = self.st.borrow_mut();
            if i >= st.tp.len() { return; }
            st.tp.remove(i); if i < st.sel.len() { st.sel.remove(i); }
        }
        self.fp_clear();
    }
    fn row_move(&self, i: usize, d: i32) {
        let ui = self.ui();
        {
            let mut st = self.st.borrow_mut();
            let j = i as i32 + d;
            if j < 0 || j as usize >= st.tp.len() || i >= st.tp.len() { return; }
            st.tp.swap(i, j as usize); st.sel.swap(i, j as usize);
            if ui.get_fp_idx() == i as i32 { ui.set_fp_idx(j); } else if ui.get_fp_idx() == j { ui.set_fp_idx(i as i32); }
        }
        self.show_rows();
    }
    fn row_select(&self, i: usize, v: bool) {
        { let mut st = self.st.borrow_mut(); if i < st.sel.len() { st.sel[i] = v; } }
        self.show_rows();
    }
    fn rows_delete_selected(&self) {
        let n = self.st.borrow().sel.iter().filter(|s| **s).count();
        if n == 0 { return self.toast("Tick the fields you want to delete first", true); }
        {
            let mut st = self.st.borrow_mut();
            let keep: Vec<bool> = st.sel.iter().map(|s| !*s).collect();
            let mut k = keep.iter();
            st.tp.retain(|_| *k.next().unwrap_or(&true));
            st.sel = vec![false; st.tp.len()];
        }
        self.fp_clear();
        self.toast(&format!("{n} field(s) removed. Press SAVE to keep the test."), false);
    }

    // ----- age and gender ranges -----
    fn ar_open(&self, add: bool) {
        let ui = self.ui();
        let rows: Vec<ARow> = {
            let st = self.st.borrow();
            if let Some(r) = &st.fp_ranges { r.iter().map(overlay_row).collect() }
            else if let Some(p) = st.tp.get(ui.get_fp_idx().max(0) as usize).filter(|_| ui.get_fp_idx() >= 0) {
                let mut v: Vec<ARow> = p.ranges.iter().map(overlay_row).collect();
                for (g, o) in &p.gender_ranges {
                    let mut o = o.clone();
                    if o.gender.is_none() && (g == "Male" || g == "Female") { o.gender = Some(g.clone()); }
                    if g == "Child" { o.age_max_days = Some(6574.0); }
                    v.push(overlay_row(&o));
                }
                v
            } else { vec![] }
        };
        let mut rows = rows;
        if add || rows.is_empty() { rows.push(blank_arow()); }
        { let mut st = self.st.borrow_mut(); st.ar = rows.clone(); st.ar_model.set_vec(rows); }
        ui.set_ar_shown(true);
    }
    fn ar_refresh(&self) { let st = self.st.borrow(); st.ar_model.set_vec(st.ar.clone()); }
    fn ar_add(&self) { self.st.borrow_mut().ar.push(blank_arow()); self.ar_refresh(); }
    fn ar_remove(&self, i: usize) { { let mut st = self.st.borrow_mut(); if i < st.ar.len() { st.ar.remove(i); } } self.ar_refresh(); }
    fn ar_edit(&self, i: usize, f: i32, t: &str) {
        let rebuild = matches!(f, 7 | 8);
        {
            let mut st = self.st.borrow_mut();
            let Some(r) = st.ar.get_mut(i) else { return };
            let v: SharedString = t.into();
            match f { 0 => r.from = v, 1 => r.to = v, 2 => r.low = v, 3 => r.high = v, 4 => r.clow = v, 5 => r.chigh = v, 6 => r.text = v, 7 => r.gender = v, _ => r.unit = v }
        }
        if rebuild { self.ar_refresh(); }
    }
    fn ar_done(&self) {
        let rows = self.st.borrow().ar.clone();
        let mut out: Vec<labdesk_core::RangeOverlay> = Vec::new();
        for (n, r) in rows.iter().enumerate() {
            let blank = [&r.from, &r.to, &r.low, &r.high, &r.clow, &r.chigh, &r.text].iter().all(|x| x.trim().is_empty());
            if blank { continue; }
            let label = format!("Row {}", n + 1);
            let parsed = (|| -> Result<_, String> { Ok((parse_num("Age from", &label, &r.from)?, parse_num("Age to", &label, &r.to)?, parse_num("Low", &label, &r.low)?, parse_num("High", &label, &r.high)?,
                parse_num("Critical below", &label, &r.clow)?, parse_num("Critical above", &label, &r.chigh)?)) })();
            let (af, at, lo, hi, cl, ch) = match parsed { Ok(v) => v, Err(m) => return self.toast(&m, true) };
            if af.zip(at).is_some_and(|(a, b)| a > b) { return self.toast(&format!("{label}: the age starts above where it ends"), true); }
            if lo.zip(hi).is_some_and(|(a, b)| a > b) { return self.toast(&format!("{label}: the low is above the high"), true); }
            let k = match r.unit.as_str() { "Days" => 1.0, "Months" => 30.4375, _ => 365.25 };
            let g = r.gender.to_string();
            out.push(labdesk_core::RangeOverlay {
                gender: if g == "Male" || g == "Female" { Some(g) } else { None },
                age_min_days: af.map(|v| (v * k * 100.0).round() / 100.0), age_max_days: at.map(|v| (v * k * 100.0).round() / 100.0),
                low: lo, high: hi, critical_low: cl, critical_high: ch,
                range_text: Some(r.text.trim().to_string()).filter(|t| !t.is_empty()), ..Default::default()
            });
        }
        let ui = self.ui();
        ui.set_fp_range_count(if out.is_empty() { "Age / gender ranges will be removed when you press the field button.".to_string() } else { format!("{} age / gender range(s) ready. Press the field button (ADD / UPDATE FIELD) to keep them.", out.len()) }.into());
        self.st.borrow_mut().fp_ranges = Some(out);
        ui.set_ar_shown(false);
    }

    // ----- opening, resetting, copying -----
    fn te_load_form(&self, t: &TestDef) {
        let ui = self.ui();
        let c = &t.config;
        ui.set_te_code(t.code.as_str().into()); ui.set_te_name(t.name.as_str().into()); ui.set_te_dept(t.department.as_str().into());
        ui.set_te_sample(t.sample.as_str().into()); ui.set_te_price(format!("{}", t.price_paise as f64 / 100.0).into()); ui.set_te_state(t.disabled as i32);
        ui.set_te_type(if c.test_type.is_empty() { "Default".into() } else { SharedString::from(c.test_type.as_str()) });
        ui.set_te_gender(match c.gender.as_str() { "Male" => 0, "Female" => 1, _ => 2 });
        ui.set_te_color(c.sample_color.as_str().into()); ui.set_te_suffix(c.barcode_suffix.as_str().into());
        ui.set_te_labid(if c.labdesk_id.is_empty() { c.extra.get("source_id").and_then(|v| v.as_str()).unwrap_or("") } else { c.labdesk_id.as_str() }.into());
        ui.set_te_lmp(c.lmp_required); ui.set_te_idreq(c.id_required); ui.set_te_consent(c.consent_form); ui.set_te_species(c.species);
        ui.set_te_billing(c.only_billing); ui.set_te_nextpage(c.start_next_page); ui.set_te_precol(c.pre_collection.as_str().into());
        ui.set_te_copy_q("".into());
        self.st.borrow().copy_model.set_vec(vec![]);
        { let mut st = self.st.borrow_mut(); st.tp = c.parameters.clone(); st.sel = vec![false; c.parameters.len()]; st.interp_plain = labdesk_core::report::strip_html(&c.interpretation); st.te_cfg = c.clone(); }
        self.fp_clear();
    }
    fn refresh_choices(&self) {
        let ui = self.ui();
        let all = self.st.borrow().store.tests().unwrap_or_default();
        let uniq = |f: &dyn Fn(&TestDef) -> String| { let mut v: Vec<String> = all.iter().map(f).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(); v.sort_by_key(|s| s.to_lowercase()); v.dedup_by(|a, b| a.eq_ignore_ascii_case(b)); v.into_iter().map(SharedString::from).collect::<Vec<_>>() };
        ui.set_dept_choices(ModelRc::from(Rc::new(VecModel::from(uniq(&|t| t.department.to_uppercase())))));
        ui.set_sample_choices(ModelRc::from(Rc::new(VecModel::from(uniq(&|t| { let s = t.sample.trim(); let mut c = s.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default() })))));
    }
    fn te_new(&self) {
        let ui = self.ui();
        self.te_errors_off();
        self.refresh_choices();
        self.st.borrow_mut().te_base = None;
        ui.set_te_id(0);
        self.te_load_form(&TestDef { id: 0, code: String::new(), name: String::new(), department: String::new(), sample: String::new(), price_paise: 0, disabled: false, config: TestConfig { gender: "Both".into(), ..Default::default() } });
        self.refresh_admin_tests();
    }
    fn te_open(&self, id: i64) {
        let ui = self.ui();
        let found = self.st.borrow().store.tests().ok().and_then(|l| l.into_iter().find(|t| t.id == id));
        let Some(t) = found else { return };
        self.te_errors_off();
        self.refresh_choices();
        ui.set_te_id(id as i32);
        self.te_load_form(&t);
        self.st.borrow_mut().te_base = Some(t);
        self.refresh_admin_tests();
    }
    fn te_back(&self) {
        let ui = self.ui();
        ui.set_te_id(-1);
        self.fp_clear();
        self.refresh_admin_tests();
    }
    fn te_reset(&self) {
        let id = self.ui().get_te_id();
        if id > 0 { self.te_open(id as i64); } else { self.te_new(); }
        self.toast("Everything you changed since the last save was undone.", false);
    }
    fn te_copy_changed(&self) {
        let q = self.ui().get_te_copy_q().trim().to_lowercase();
        let all = self.st.borrow().store.tests().unwrap_or_default();
        let hits: Vec<&TestDef> = if q.is_empty() { vec![] } else { all.iter().filter(|t| t.name.to_lowercase().contains(&q) || t.code.to_lowercase().contains(&q)).take(6).collect() };
        let mut st = self.st.borrow_mut();
        st.copy_ids = hits.iter().map(|t| t.id).collect();
        st.copy_model.set_vec(hits.iter().map(|t| TestAdminRow { id: t.id as i32, code: t.code.as_str().into(), name: t.name.as_str().into(), dept: t.department.as_str().into(), price: "".into(), off: false }).collect::<Vec<_>>());
    }
    fn te_copy_pick(&self, i: usize) {
        let ui = self.ui();
        let Some(id) = self.st.borrow().copy_ids.get(i).copied() else { return };
        let Some(t) = self.st.borrow().store.tests().ok().and_then(|l| l.into_iter().find(|t| t.id == id)) else { return };
        let n = t.config.parameters.len();
        {
            let mut st = self.st.borrow_mut();
            st.tp = t.config.parameters.clone(); st.sel = vec![false; n];
            st.te_cfg.comment = t.config.comment.clone(); st.te_cfg.interpretation = t.config.interpretation.clone(); st.interp_plain = labdesk_core::report::strip_html(&t.config.interpretation);
            st.copy_model.set_vec(vec![]); st.copy_ids.clear();
        }
        ui.set_te_copy_q("".into());
        self.fp_clear();
        self.toast(&format!("Copied {n} field(s) from {}. Change what you need, then press SAVE.", t.name), false);
    }

    // ----- comment, interpretation, pre-collection text -----
    fn te_open_text(&self, kind: i32) {
        let ui = self.ui();
        let (title, text) = {
            let mut st = self.st.borrow_mut();
            st.ci_kind = kind;
            match kind {
                0 => ("Comment printed under this test on the report", st.te_cfg.comment.clone()),
                1 => ("Interpretation (kept with the test)", st.interp_plain.clone()),
                _ => ("Pre-collection guidelines for the patient", st.te_cfg.pre_collection.clone()),
            }
        };
        ui.set_ci_title(title.into()); ui.set_ci_text(text.into()); ui.set_ci_shown(true);
    }
    fn ci_save(&self) {
        let ui = self.ui();
        let text = ui.get_ci_text().to_string();
        {
            let mut st = self.st.borrow_mut();
            match st.ci_kind {
                0 => st.te_cfg.comment = text.trim().to_string(),
                1 => { if text.trim() != st.interp_plain.trim() { st.te_cfg.interpretation = text.trim().to_string(); st.interp_plain = text.trim().to_string(); } }
                _ => st.te_cfg.pre_collection = text.trim().to_string(),
            }
            let c = st.te_cfg.pre_collection.clone();
            drop(st);
            ui.set_te_precol(c.as_str().into());
        }
        ui.set_ci_shown(false);
        self.toast("Kept. Press SAVE (top right) to save the test.", false);
    }

    // ----- save and preview -----
    fn collect_test(&self) -> Option<TestDef> {
        let ui = self.ui();
        self.te_errors_off();
        let (code, name, dept) = (ui.get_te_code().trim().to_string(), ui.get_te_name().trim().to_string(), ui.get_te_dept().trim().to_string());
        let mut bad = false;
        if code.is_empty() { ui.set_err_te_code("Required".into()); bad = true; }
        if name.is_empty() { ui.set_err_te_name("Please enter the test name".into()); bad = true; }
        if dept.is_empty() { ui.set_err_te_dept("Please enter the department".into()); bad = true; }
        let price = fmt::parse_rupees(&ui.get_te_price());
        if price.is_none() { ui.set_err_te_price("Numbers only".into()); bad = true; }
        if bad { self.toast("Please check the highlighted fields", true); return None; }
        let (base, mut cfg, tp) = { let st = self.st.borrow(); (st.te_base.clone(), st.te_cfg.clone(), st.tp.clone()) };
        cfg.parameters = tp;
        cfg.gender = match ui.get_te_gender() { 0 => "Male", 1 => "Female", _ => "Both" }.into();
        cfg.test_type = ui.get_te_type().to_string(); cfg.sample_color = ui.get_te_color().trim().to_string(); cfg.barcode_suffix = ui.get_te_suffix().trim().to_string();
        cfg.labdesk_id = ui.get_te_labid().trim().to_string();
        cfg.lmp_required = ui.get_te_lmp(); cfg.id_required = ui.get_te_idreq(); cfg.consent_form = ui.get_te_consent(); cfg.species = ui.get_te_species();
        cfg.only_billing = ui.get_te_billing(); cfg.start_next_page = ui.get_te_nextpage();
        Some(TestDef { id: base.as_ref().map_or(0, |b| b.id), code, name, department: dept, sample: ui.get_te_sample().trim().to_string(), price_paise: price.unwrap_or(0), disabled: ui.get_te_state() == 1, config: cfg })
    }
    fn te_save(&self) {
        let ui = self.ui();
        let Some(def) = self.collect_test() else { return };
        let actor = self.actor();
        let result = self.st.borrow_mut().store.save_test(&actor, &def);
        match result {
            Ok(t) => {
                self.refresh_tests();
                self.te_open(t.id);
                self.toast("Test saved. New patients will use it; reports already made are not changed.", false);
            }
            Err(e) => {
                let m = friendly(&e);
                if m.contains("already uses") { ui.set_err_te_code("This code is already used by another test".into()); }
                self.toast(&m, true);
            }
        }
    }
    fn te_preview(&self) {
        let Some(def) = self.collect_test() else { return };
        let letterhead = self.ui().get_use_letterhead();
        let result = self.st.borrow().store.test_preview_pdf(&def, letterhead);
        match result {
            Ok(r) => {
                let path = std::env::temp_dir().join(format!("LabDesk preview {}.pdf", safe_name(&def.code)));
                match std::fs::write(&path, &r.pdf) {
                    Ok(()) => { if self.opts.open_files { let _ = open::that(&path); } self.toast("Preview made with made-up values. Nothing was saved.", false); }
                    Err(e) => self.toast(&format!("Could not make the preview: {e}"), true),
                }
            }
            Err(e) => self.fail(e),
        }
    }
    fn pick_design(&self, i: i32) {
        let ui = self.ui();
        let existing = match self.st.borrow().store.profile() { Ok(p) => p, Err(e) => return self.fail(e) };
        let p = Profile { header_design: i.clamp(0, 7) as u8, ..existing };
        let actor = self.actor();
        let result = self.st.borrow_mut().store.save_profile(&actor, &p);
        match result {
            Ok(()) => { ui.set_s_design(i); self.toast("Design saved. Press PRINT A TEST PAGE to see it.", false); }
            Err(e) => self.fail(e),
        }
    }
}

// ---------- bill, patient details, delete ----------
impl Ctl {
    fn make_bill(&self) { self.make_bill_with(false); }
    fn make_bill_with(&self, print: bool) {
        let id = match &self.st.borrow().order { Some(o) => o.id, None => return };
        let ui = self.ui();
        let (header, roll) = (ui.get_bill_header(), match ui.get_bill_paper() { 1 => Some(80.0f32), 2 => Some(58.0), _ => None });
        let actor = self.actor();
        let result = self.st.borrow_mut().store.bill_pdf_on(&actor, id, header, roll);
        match result {
            Ok(r) => {
                let (code, name) = { let st = self.st.borrow(); let o = st.order.as_ref().expect("order open"); (o.patient.patient_code.clone(), format!("{} {}", o.patient.first_name, o.patient.last_name)) };
                let tag = match roll { Some(mm) => format!(" roll {mm:.0}mm"), None => String::new() };
                match self.save_pdf(&safe_name(&format!("Bill {code} {name}{tag}")), &r.pdf) { Ok(p) => if print { self.print_saved(&p, "Bill") } else { self.show_pdf(&p, "Bill") }, Err(m) => self.toast(&m, true) }
            }
            Err(e) => self.fail(e),
        }
    }

    /// Remember the bill header and paper choices for the whole lab.
    fn bill_prefs_save(&self) {
        let ui = self.ui();
        let prefs = labdesk_core::BillPrefs { header: ui.get_bill_header(), paper: match ui.get_bill_paper() { 1 => "roll80", 2 => "roll58", _ => "a4" }.into() };
        let actor = self.actor();
        if let Err(e) = self.st.borrow_mut().store.set_bill_prefs(&actor, &prefs) { self.fail(e); }
    }
    fn bill_prefs_load(&self) {
        let prefs = self.st.borrow().store.bill_prefs().unwrap_or_default();
        let ui = self.ui();
        ui.set_bill_header(prefs.header);
        ui.set_bill_paper(match prefs.paper.as_str() { "roll80" => 1, "roll58" => 2, _ => 0 });
    }

    /// Names of doctors typed before, as small buttons under the doctor's name box.
    fn doctor_edited(&self, typed: &str) {
        let t = typed.trim();
        let names = self.st.borrow().store.referral_names(t, 12).unwrap_or_default();
        let list: Vec<SharedString> = names.into_iter().filter(|n| t.is_empty() || !n.eq_ignore_ascii_case(t)).take(4).map(SharedString::from).collect();
        self.st.borrow().doctors.set_vec(list);
    }

    fn edit_patient(&self) {
        let ui = self.ui();
        let Some(o) = self.st.borrow().order.clone() else { return };
        let p = &o.patient;
        let delivered = o.status == "Delivered";
        let approved = o.tests.iter().any(|t| t.is_approved());
        for f in [AppWindow::set_err_e_first, AppWindow::set_err_e_age, AppWindow::set_err_e_address, AppWindow::set_err_e_email, AppWindow::set_err_e_doctor] { f(&ui, "".into()); }
        ui.set_e_first(p.first_name.as_str().into()); ui.set_e_last(p.last_name.as_str().into()); ui.set_e_age(num_text(Some(p.age)));
        ui.set_e_age_type(["Year", "Month", "Day"].iter().position(|x| *x == p.age_type).unwrap_or(0) as i32);
        ui.set_e_gender(["Male", "Female", "Other"].iter().position(|x| *x == p.gender).unwrap_or(0) as i32);
        ui.set_e_phone(p.phone.as_str().into()); ui.set_e_email(p.email.as_str().into()); ui.set_e_address(p.address.as_str().into());
        ui.set_e_ref(if o.referral_name.is_empty() { 0 } else { 1 }); ui.set_e_doctor(o.referral_name.as_str().into());
        ui.set_e_lock_name(delivered); ui.set_e_lock_body(delivered || approved); ui.set_e_lock_ref(delivered);
        ui.set_e_note(if delivered { "This report was handed to the patient, so it is locked. Only the phone number, email and address can still be changed." }
            else if approved { "A test is already approved, so age and gender are locked (they decide the normal ranges). To change them, the doctor must first press CORRECT." } else { "" }.into());
        self.doctor_edited("");
        ui.set_edit_shown(true);
    }

    fn edit_save(&self) {
        let ui = self.ui();
        let Some(o) = self.st.borrow().order.clone() else { return };
        let mut bad = false;
        let mut flag = |cond: bool, set: &dyn Fn(&str), msg: &str| { if cond { set(msg); bad = true; } else { set(""); } };
        flag(ui.get_e_first().trim().is_empty(), &|m| ui.set_err_e_first(m.into()), "Please enter the patient's name");
        let age: Option<f64> = ui.get_e_age().trim().parse().ok().filter(|a: &f64| a.is_finite() && *a >= 0.0);
        flag(age.is_none(), &|m| ui.set_err_e_age(m.into()), "The age must be a number, for example 34");
        flag(ui.get_e_address().trim().is_empty(), &|m| ui.set_err_e_address(m.into()), "Please enter the patient's address");
        let email_err = labdesk_core::check_email(&ui.get_e_email()).err().map(|e| friendly(&e));
        flag(email_err.is_some(), &|m| ui.set_err_e_email(m.into()), email_err.as_deref().unwrap_or(""));
        let by_doctor = ui.get_e_ref() == 1;
        flag(by_doctor && ui.get_e_doctor().trim().is_empty(), &|m| ui.set_err_e_doctor(m.into()), "Please type the doctor's name, or choose Self");
        if bad { return; }
        let gender: String = ["Male", "Female", "Other"][ui.get_e_gender().clamp(0, 2) as usize].into();
        let new = NewPatient {
            designation: if gender == o.patient.gender { o.patient.designation.clone() } else { String::new() },
            first_name: ui.get_e_first().into(), last_name: ui.get_e_last().into(), age: age.unwrap_or(0.0),
            age_type: ["Year", "Month", "Day"][ui.get_e_age_type().clamp(0, 2) as usize].into(), gender,
            phone: ui.get_e_phone().into(), email: ui.get_e_email().into(), address: ui.get_e_address().into(),
        };
        let referral = if by_doctor { Referral::Doctor(ui.get_e_doctor().into()) } else { Referral::SelfReferred };
        let actor = self.actor();
        let result = self.st.borrow_mut().store.update_patient(&actor, o.id, &new, &referral);
        match result {
            Ok(fresh) => {
                ui.set_edit_shown(false);
                self.st.borrow_mut().order = Some(fresh);
                self.rebuild_rows(); self.refresh_header(); self.refresh_orders(); self.refresh_home();
                self.toast("Patient details saved", false);
            }
            Err(Error::Rule(m)) => { ui.set_err_e_first(if m.to_lowercase().contains("name") { m.as_str() } else { "" }.into()); self.toast(&m, true); }
            Err(e) => self.fail(e),
        }
    }

    fn ask_delete(&self) {
        let Some(o) = self.st.borrow().order.clone() else { return };
        let ui = self.ui();
        self.ask(Pending::DeleteOrder(o.id), &format!("Delete the report of {}?", o.patient.first_name),
            "The report disappears from all lists and from the money totals. Nothing is destroyed: an Admin can bring it back from Settings > Data & backup.\n\nTo make sure it is really you, please type your own password.", "YES, DELETE", true);
        ui.set_confirm_extra_label("Why are you deleting it?".into());
    }
    fn delete_order(&self, id: i64, reason: &str) {
        let actor = self.actor();
        let result = self.st.borrow_mut().store.delete_order(&actor, id, reason);
        match result {
            Ok(()) => {
                { let mut st = self.st.borrow_mut(); st.order = None; st.entries.clear(); st.params.set_vec(vec![]); }
                self.ui().set_page(2);
                self.refresh_orders(); self.refresh_home(); self.refresh_data();
                self.toast("The report was deleted. An Admin can bring it back from Settings > Data & backup.", false);
            }
            Err(e) => self.fail(e),
        }
    }
    fn restore_deleted(&self, id: i64) {
        let actor = self.actor();
        let result = self.st.borrow_mut().store.restore_order(&actor, id);
        match result { Ok(()) => { self.refresh_data(); self.refresh_orders(); self.refresh_home(); self.toast("The report is back in the Reports list", false); } Err(e) => self.fail(e) }
    }
}

// ---------- backup and restore ----------
impl Ctl {
    fn backups_dir(&self) -> PathBuf { self.opts.docs_dir.join("Backups") }

    /// Every lab gets daily backups from day one: if no folder was ever chosen, use Documents\LabDesk\Backups.
    fn default_backup_folder(&self) {
        if !self.st.borrow().user.as_ref().is_some_and(|u| u.role == Role::Admin) { return; }
        let cfg = match self.st.borrow().store.backup_settings() { Ok(c) => c, Err(_) => return };
        if !cfg.folder.is_empty() { return; }
        let dir = self.backups_dir();
        if std::fs::create_dir_all(&dir).is_err() { return; }
        let actor = self.actor();
        let _ = self.st.borrow_mut().store.save_backup_settings(&actor, &BackupSettings { folder: dir.display().to_string(), ..cfg });
    }

    pub fn auto_backup(&self) {
        if self.st.borrow().user.is_none() { return; }
        let r = self.st.borrow_mut().store.auto_backup();
        match r {
            Ok(Some((_, failed))) if !failed.is_empty() => self.toast(&format!("Today's automatic backup had a problem. {}", failed.join(" ")), true),
            Err(e) => self.fail(e),
            _ => {}
        }
        self.refresh_data();
    }

    /// Fill the Data & backup screen: the backup settings, the last backup, and the deleted reports.
    pub fn refresh_data(&self) {
        let ui = self.ui();
        let st = self.st.borrow();
        if !st.user.as_ref().is_some_and(|u| u.role == Role::Admin) { return; }
        if let Ok(cfg) = st.store.backup_settings() {
            ui.set_bk_folder(cfg.folder.as_str().into()); ui.set_bk_cloud(cfg.cloud_folder.as_str().into());
            ui.set_bk_encrypt(cfg.encrypt as i32); ui.set_bk_password(cfg.password.as_str().into());
        }
        ui.set_bk_last(match st.store.last_backup() { Ok(Some(t)) => format!("Last backup: {}", fmt::when(&t)), _ => "No backup has been made yet.".to_string() }.into());
        let rows = st.store.deleted_orders().unwrap_or_default().into_iter().map(|d| DeletedRow { id: d.id as i32, name: d.patient_name.as_str().into(), code: d.patient_code.as_str().into(),
            note: format!("Deleted {} by {}. Reason: {}", fmt::when(&d.deleted_at), d.deleted_by, d.reason).into(), total: fmt::rupees(d.total_paise).into() }).collect::<Vec<_>>();
        st.deleted.set_vec(rows);
    }

    fn choose_folder(&self, cloud: bool) {
        let ui = self.ui();
        let title = if cloud { "Choose the folder that Google Drive / OneDrive / Dropbox keeps in sync" } else { "Choose the folder for backups (a USB drive is best)" };
        let Some(dir) = rfd::FileDialog::new().set_title(title).pick_folder() else { return };
        if cloud { ui.set_bk_cloud(dir.display().to_string().into()); } else { ui.set_bk_folder(dir.display().to_string().into()); }
    }

    /// Returns true when saved.
    fn save_backup_settings(&self) -> bool {
        let ui = self.ui();
        ui.set_err_bk_password("".into());
        let keep = self.st.borrow().store.backup_settings().map(|c| c.keep).unwrap_or(30);
        let cfg = BackupSettings { folder: ui.get_bk_folder().into(), cloud_folder: ui.get_bk_cloud().into(), encrypt: ui.get_bk_encrypt() == 1, password: ui.get_bk_password().into(), keep };
        let actor = self.actor();
        let result = self.st.borrow_mut().store.save_backup_settings(&actor, &cfg);
        match result {
            Ok(()) => { self.toast("Backup settings saved", false); true }
            Err(e) => { let m = friendly(&e); if m.to_lowercase().contains("password") { ui.set_err_bk_password(m.into()); } else { self.toast(&m, true); } false }
        }
    }

    pub fn backup_now(&self) {
        let ui = self.ui();
        if ui.get_bk_folder().trim().is_empty() && ui.get_bk_cloud().trim().is_empty() { ui.set_bk_folder(self.backups_dir().display().to_string().into()); let _ = std::fs::create_dir_all(self.backups_dir()); }
        if !self.save_backup_settings() { return; }
        let actor = self.actor();
        let result = self.st.borrow_mut().store.backup_now(&actor);
        match result {
            Ok((done, failed)) => {
                self.refresh_data();
                if !failed.is_empty() { self.toast(&failed.join(" "), true); }
                else if let Some(first) = done.first() {
                    let dir = first.parent().map(|p| p.display().to_string()).unwrap_or_default();
                    self.toast(&if done.len() > 1 { format!("Backup saved in {dir} and copied to your cloud folder") } else { format!("Backup saved in {dir}") }, false);
                }
            }
            Err(e) => self.fail(e),
        }
    }

    /// Pick a backup file, then ask "Are you sure?". `first_run`: from the very first screen, before any account exists.
    fn ask_restore(&self, first_run: bool) {
        let mut dlg = rfd::FileDialog::new().set_title("Choose a LabDesk backup file").add_filter("LabDesk backup", &["db", "ldbk"]);
        let dir = self.backups_dir();
        if dir.is_dir() { dlg = dlg.set_directory(dir); }
        let Some(file) = dlg.pick_file() else { return };
        self.ask_restore_file(file, first_run);
    }
    pub fn ask_restore_file(&self, file: PathBuf, first_run: bool) {
        let locked = labdesk_core::backup::is_encrypted_backup(&file);
        let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.st.borrow_mut().restore_file = Some(file);
        let ui = self.ui();
        if first_run {
            self.ask(Pending::RestoreFirstRun, "Restore your lab from this backup?", &format!("File: {name}\n\nAll the patients, reports, tests and people in the backup will be loaded. Afterwards sign in with the username and password you used before."), "YES, RESTORE", false);
        } else {
            self.ask(Pending::Restore, "Restore from this backup?", &format!("File: {name}\n\nEverything now in LabDesk will be replaced by what is in the backup. A safety copy of today's data is saved first. You will be signed out, and must sign in with the people and passwords that the backup had.\n\nPlease type your own password to confirm."), "YES, RESTORE", true);
        }
        if locked { ui.set_confirm_extra_label("Backup password".into()); ui.set_confirm_extra_secret(true); }
    }
    fn do_restore(&self, first_run: bool, password: &str) {
        let Some(file) = self.st.borrow_mut().restore_file.take() else { return };
        let pw = if password.is_empty() { None } else { Some(password) };
        let ui = self.ui();
        if first_run {
            let r = self.st.borrow_mut().store.restore_on_first_run(&file, pw);
            match r {
                Ok(()) => { ui.set_auth_first_run(!self.st.borrow().store.has_users().unwrap_or(false)); self.toast("Your lab was restored. Please sign in with the username and password you used before.", false); }
                Err(e) => self.fail(e),
            }
            return;
        }
        let actor = self.actor();
        let safety = self.backups_dir();
        let _ = std::fs::create_dir_all(&safety);
        let r = self.st.borrow_mut().store.restore_from(&actor, &file, pw, &safety);
        match r {
            Ok(kept) => {
                self.sign_out();
                ui.set_auth_first_run(!self.st.borrow().store.has_users().unwrap_or(false));
                self.toast(&format!("Restored from the backup. Today's data was saved first as {}. Please sign in.", kept.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()), false);
            }
            Err(e) => self.fail(e),
        }
    }
}

// ---------- help, problem report, links ----------
impl Ctl {
    pub fn refresh_help(&self, q: &str) {
        let rows = diag::search(q).into_iter().map(|(t, _, b)| HelpRow { title: (*t).into(), body: (*b).into() }).collect::<Vec<_>>();
        self.st.borrow().help.set_vec(rows);
    }
    fn problem_report(&self) {
        let recent: Vec<String> = self.st.borrow().store.audit_log(30).unwrap_or_default().into_iter().map(|(t, who, what, ent, id)| format!("{t}  {who}  {what} {ent} {id}")).collect();
        let text = diag::problem_report(&self.opts.data_dir, &self.opts.db_path, &recent);
        let dir = self.opts.docs_dir.join("Problem reports");
        let file = dir.join(format!("LabDesk-problem-report-{}.txt", chrono::Local::now().format("%Y-%m-%d_%H%M%S")));
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&file, text)) {
            Ok(()) => { self.open_path(&file); self.toast(&format!("Problem report saved in {}. Please send that file to your software provider.", dir.display()), false); }
            Err(e) => self.toast(&format!("Could not save the report: {e}"), true),
        }
    }
    fn open_url(&self, url: &str) { if self.opts.open_files { let _ = open::that(url); } }
    fn open_path(&self, p: &Path) { if self.opts.open_files { let _ = open::that(p); } }
}

// ---------- patients: history and returning patients ----------
fn pat_row(p: &labdesk_core::PatientSummary) -> PatRow {
    PatRow { id: p.id as i32, name: p.name.as_str().into(),
        meta: format!("{} · {} · {}", p.patient_code, p.age_gender, if p.phone.trim().is_empty() { "no phone" } else { p.phone.as_str() }).into(),
        code: p.patient_code.as_str().into(), visits: if p.visits == 1 { "1 visit".to_string() } else { format!("{} visits", p.visits) }.into(), last: fmt::day(&p.last_visit).into() }
}

impl Ctl {
    pub fn refresh_patients(&self) {
        let ui = self.ui();
        let q = ui.get_pat_query().to_string();
        if q.contains("Report no:") { ui.set_pat_query("".into()); if self.try_scan(&q) { return; } }
        let q = ui.get_pat_query().to_string();
        let r = self.st.borrow().store.patients(&q, 300);
        match r {
            Ok(list) => { self.st.borrow().pats.set_vec(list.iter().map(pat_row).collect::<Vec<_>>()); }
            Err(e) => self.fail(e),
        }
        let sel = ui.get_pat_selected();
        if sel != 0 { self.show_patient(sel as i64); }
    }

    /// Fill the right-hand side of the Patients screen: summary, tests done, and the tree of visits.
    pub fn show_patient(&self, id: i64) {
        let ui = self.ui();
        let h = match self.st.borrow().store.patient_history(id) { Ok(h) => h, Err(_) => { ui.set_pat_selected(0); return; } };
        let p = &h.patient;
        ui.set_pat_selected(id as i32);
        self.trend_load(id);
        ui.set_pat_name(format!("{} {} {}", p.designation, p.first_name, p.last_name).split_whitespace().collect::<Vec<_>>().join(" ").into());
        ui.set_pat_meta(format!("ID {} · {} {} · {}", p.patient_code, p.age, p.age_type.to_lowercase(), p.gender).into());
        let contact: Vec<&str> = [p.phone.as_str(), p.email.as_str(), p.address.as_str()].into_iter().filter(|x| !x.trim().is_empty()).collect();
        ui.set_pat_contact(contact.join("  ·  ").into());
        ui.set_pat_visits(h.visits.len().to_string().into());
        ui.set_pat_first(fmt::day(&h.first_visit).into()); ui.set_pat_last(fmt::day(&h.last_visit).into());
        ui.set_pat_billed(fmt::rupees(h.billed_paise).into()); ui.set_pat_paid(fmt::rupees(h.paid_paise).into());
        ui.set_pat_due(if h.due_paise > 0 { fmt::rupees(h.due_paise) } else { "Nothing".to_string() }.into()); ui.set_pat_has_due(h.due_paise > 0);
        ui.set_pat_can_pay(h.due_paise > 0 && matches!(self.st.borrow().user.as_ref().map(|u| u.role), Some(Role::Admin | Role::Reception)));
        let st = self.st.borrow();
        let pays = self.st.borrow().store.patient_payments(id).unwrap_or_default();
        ui.set_pat_pays_total(fmt::rupees(pays.iter().map(|x| x.amount_paise).sum()).into());
        st.pat_pays.set_vec(pays.iter().map(|x| PayRow { when: fmt::when_full(&x.created_at).into(),
            how: if x.reference.trim().is_empty() { x.method.clone() } else { format!("{}  ({})", x.method, x.reference.trim()) }.into(),
            bill: format!("for B{:05}", x.order_id).into(), amount: fmt::rupees(x.amount_paise).into() }).collect::<Vec<_>>());
        st.pat_tests.set_vec(h.test_counts.iter().map(|c| CountRow { name: c.name.as_str().into(), times: format!("× {}", c.times).into(), last: format!("last {}", fmt::day(&c.last)).into() }).collect::<Vec<_>>());
        let mut tree = Vec::new();
        for v in &h.visits {
            let due = v.total_paise - v.paid_paise;
            tree.push(TreeRow { kind: 0, order_id: v.order_id as i32, title: fmt::when_full(&v.created_at).into(),
                sub: format!("Age {} · {} · {} test{}", v.age_text, if v.referral.is_empty() { "came by themselves".to_string() } else { format!("sent by {}", v.referral) }, v.tests.len(), if v.tests.len() == 1 { "" } else { "s" }).into(),
                right: if due > 0 { format!("{}  ·  due {}", fmt::rupees(v.total_paise), fmt::rupees(due)) } else { fmt::rupees(v.total_paise) }.into(), status: v.status.as_str().into(), tone: if due > 0 { 3 } else { 0 } });
            for t in &v.tests {
                let (right, tone) = if t.status == "Completed" { if t.abnormal > 0 { (format!("{} outside normal", t.abnormal), 3) } else { ("all normal".to_string(), 1) } }
                    else if t.entered > 0 { ("in progress".to_string(), 2) } else { ("waiting for result".to_string(), 0) };
                tree.push(TreeRow { kind: 1, order_id: v.order_id as i32, title: t.name.as_str().into(), sub: "".into(), right: right.into(), status: "".into(), tone });
            }
        }
        st.pat_tree.set_vec(tree);
    }

    /// From the Patients screen: start a new visit for this person.
    fn new_visit(&self) {
        let id = self.ui().get_pat_selected() as i64;
        if id == 0 { return; }
        self.navigate(1);
        self.pick_patient(id);
    }

    pub fn find_edited(&self, q: &str) {
        if q.contains("Report no:") { self.ui().set_f_find("".into()); if self.try_scan(q) { return; } }
        let list = if q.trim().chars().count() < 2 { vec![] } else { self.st.borrow().store.patients(q, 5).unwrap_or_default() };
        self.st.borrow().finds.set_vec(list.iter().map(pat_row).collect::<Vec<_>>());
    }

    /// Fill the registration form from a patient who has been here before.
    pub fn pick_patient(&self, id: i64) {
        let ui = self.ui();
        let h = match self.st.borrow().store.patient_history(id) { Ok(h) => h, Err(e) => { self.fail(e); return; } };
        let p = &h.patient;
        ui.set_f_first(p.first_name.as_str().into()); ui.set_f_last(p.last_name.as_str().into());
        ui.set_f_age(format!("{}", p.age).into());
        ui.set_f_age_type(match p.age_type.as_str() { "Month" => 1, "Day" => 2, _ => 0 });
        ui.set_f_gender(match p.gender.as_str() { "Female" => 1, "Other" => 2, _ => 0 });
        ui.set_f_phone(p.phone.as_str().into()); ui.set_f_email(p.email.as_str().into()); ui.set_f_address(p.address.as_str().into());
        for e in [AppWindow::set_err_first as fn(&AppWindow, SharedString), AppWindow::set_err_age, AppWindow::set_err_address, AppWindow::set_err_email] { e(&ui, "".into()); }
        ui.set_f_linked(id as i32);
        ui.set_f_linked_text(format!("{} {} {} · ID {} · {} earlier visit{} · last {}", p.designation, p.first_name, p.last_name, p.patient_code, h.visits.len(), if h.visits.len() == 1 { "" } else { "s" }, fmt::day(&h.last_visit)).split_whitespace().collect::<Vec<_>>().join(" ").into());
        ui.set_f_find("".into());
        self.st.borrow().finds.set_vec(vec![]);
        self.st.borrow_mut().linked = Some(id);
        self.st.borrow().matches.set_vec(vec![]);
        self.toast(&format!("Details filled in. Please check the age: it was {} {} when {} last came.", p.age, p.age_type.to_lowercase(), p.first_name), false);
    }

    fn unlink_patient(&self) {
        let ui = self.ui();
        self.clear_link();
        for s in [AppWindow::set_f_first as fn(&AppWindow, SharedString), AppWindow::set_f_last, AppWindow::set_f_age, AppWindow::set_f_phone, AppWindow::set_f_email, AppWindow::set_f_address] { s(&ui, "".into()); }
        ui.set_f_age_type(0); ui.set_f_gender(0);
    }
    fn clear_link(&self) {
        let ui = self.ui();
        self.st.borrow_mut().linked = None;
        self.st.borrow_mut().skip_match = false;
        self.st.borrow().matches.set_vec(vec![]);
        ui.set_f_linked(0); ui.set_f_linked_text("".into()); ui.set_f_find("".into());
        self.st.borrow().finds.set_vec(vec![]);
    }
}

// ---------- same-person hints, paying later, scanned reports ----------
impl Ctl {
    /// While a new patient is being typed: show people already registered with the same phone or a similar name.
    pub fn match_edited(&self) {
        let ui = self.ui();
        if self.st.borrow().linked.is_some() || ui.get_page() != 1 { self.st.borrow().matches.set_vec(vec![]); return; }
        let list = self.st.borrow().store.possible_matches(&ui.get_f_first(), &ui.get_f_last(), &ui.get_f_phone(), 3).unwrap_or_default();
        self.st.borrow().matches.set_vec(list.iter().map(pat_row).collect::<Vec<_>>());
    }

    /// Text typed into a search box that looks like a scanned report QR. True when it was a report of this lab.
    pub fn try_scan(&self, text: &str) -> bool {
        let v = self.st.borrow().store.verify_report_text(text);
        let v = match v { Ok(v) => v, Err(e) => { self.fail(e); return true; } };
        let Some(v) = v else { self.toast("This QR code is not from a report of this lab.", true); return false };
        let pid = match self.st.borrow().store.order(v.order_id) { Ok(o) => o.patient.id, Err(_) => return false };
        self.ui().set_pat_query("".into());
        self.navigate(6);
        self.show_patient(pid);
        let ui = self.ui();
        ui.set_verdict_order(v.order_id as i32);
        if v.genuine {
            ui.set_verdict_text(format!("GENUINE: this printout of {} matches the report the doctor approved. Check the values on the paper against the report.", v.patient_name).into());
            ui.set_verdict_kind(1);
        } else {
            ui.set_verdict_text(format!("WARNING: this printout of {} does NOT match the lab's record. Do not accept it as genuine.", v.patient_name).into());
            ui.set_verdict_kind(2);
        }
        true
    }

    fn plain_rupees(paise: i64) -> String { if paise % 100 == 0 { (paise / 100).to_string() } else { format!("{}.{:02}", paise / 100, paise % 100) } }

    fn open_pay(&self, target: PayTarget, title: String, text: String, due: i64) {
        let ui = self.ui();
        self.st.borrow_mut().pay_target = Some(target);
        ui.set_pay_title(title.into()); ui.set_pay_text(text.into());
        ui.set_pay_amount(Self::plain_rupees(due).into()); ui.set_pay_method(0); ui.set_pay_ref("".into()); ui.set_err_pay("".into());
        ui.set_pay_shown(true);
    }
    /// From the result sheet: take money for this visit's bill.
    pub fn pay_open(&self) {
        let Some(o) = self.st.borrow().order.clone() else { return };
        if o.due_paise() <= 0 { return; }
        let name = format!("{} {}", o.patient.first_name, o.patient.last_name).trim().to_string();
        self.open_pay(PayTarget::Order(o.id), "Receive payment".into(),
            format!("{name}\nBill {}  ·  paid {}  ·  still due {}", fmt::rupees(o.total_paise), fmt::rupees(o.paid_paise()), fmt::rupees(o.due_paise())), o.due_paise());
    }
    /// From the Patients screen: take money against everything the patient owes (oldest visit first).
    pub fn pay_open_patient(&self) {
        let id = self.ui().get_pat_selected() as i64;
        let due = self.st.borrow().store.patient_due(id).unwrap_or(0);
        if id == 0 || due <= 0 { return; }
        let name = self.ui().get_pat_name().to_string();
        self.open_pay(PayTarget::Patient(id), "Receive payment".into(),
            format!("{name}\nStill due over all visits: {}. The money is counted against the oldest visit first.", fmt::rupees(due)), due);
    }
    pub fn pay_save(&self) {
        let ui = self.ui();
        let Some(target) = self.st.borrow().pay_target else { return ui.set_pay_shown(false) };
        let amount = match fmt::parse_rupees(&ui.get_pay_amount()) { Some(a) if a > 0 => a, _ => { ui.set_err_pay("Please type the amount received, for example 150 or 150.50".into()); return; } };
        let method = ["Cash", "UPI", "Card", "Bank"][(ui.get_pay_method().clamp(0, 3)) as usize];
        let note = ui.get_pay_ref().to_string();
        let actor = self.actor();
        let result: Result<(), Error> = match target {
            PayTarget::Order(id) => self.st.borrow_mut().store.add_payment(&actor, id, amount, method, &note).map(|_| ()),
            PayTarget::Patient(id) => self.st.borrow_mut().store.receive_patient_payment(&actor, id, amount, method, &note).map(|_| ()),
        };
        match result {
            Ok(()) => {
                ui.set_pay_shown(false);
                self.st.borrow_mut().pay_target = None;
                match target {
                    PayTarget::Order(id) => { self.open_order(id); }
                    PayTarget::Patient(id) => { self.show_patient(id); }
                }
                self.refresh_home();
                self.toast(&format!("{} received by {}. Thank you.", fmt::rupees(amount), method), false);
            }
            Err(Error::Rule(m)) if m.to_lowercase().contains("invalid payment") || m.to_lowercase().contains("more than") => ui.set_err_pay("That is more than what is still due. Please check the amount.".into()),
            Err(e) => { ui.set_err_pay(friendly(&e).into()); }
        }
    }
}

impl Ctl {
    /// The text printed in a report's QR code (used by the screenshot tool and tests to simulate a scan).
    #[doc(hidden)]
    pub fn qr_text_for(&self, order_id: i64) -> Option<String> {
        let st = self.st.borrow();
        let o = st.store.order(order_id).ok()?;
        let profile = st.store.profile().ok()?;
        let reported = o.tests.iter().filter_map(|t| t.approved_at.clone()).max()?;
        Some(labdesk_core::report::qr_text(&profile, &o, &reported, &labdesk_core::report::verification_code(&o)))
    }
}

// ---------- one bill for everything, and the scan screen ----------
impl Ctl {
    /// Patients screen: ONE bill with all visits, all tests, all payments and the balance.
    fn print_full_bill(&self) {
        let id = self.ui().get_pat_selected() as i64;
        if id == 0 { return; }
        let letterhead = self.ui().get_bill_header();
        let actor = self.actor();
        let result = self.st.borrow_mut().store.patient_bill_pdf(&actor, id, letterhead);
        match result {
            Ok(r) => {
                let name = self.ui().get_pat_name().to_string();
                match self.save_pdf(&safe_name(&format!("Full bill {name}")), &r.pdf) { Ok(p) => self.show_pdf(&p, "Combined bill"), Err(m) => self.toast(&m, true) }
            }
            Err(e) => self.fail(e),
        }
    }

    pub fn scan_clear(&self) {
        let ui = self.ui();
        ui.set_scan_input("".into());
        ui.set_sc_kind(0);
        self.st.borrow().sc_rows.set_vec(vec![]);
    }
    /// Called for every key the scanner "types". A finished LabDesk code ends with "Ref:" and ten letters/digits,
    /// so the report opens by itself even if the scanner does not press Enter.
    pub fn scan_edited(&self, text: &str) {
        if let Some(i) = text.rfind("Ref:") {
            let tail = text[i + 4..].trim();
            if tail.len() >= 10 && tail.chars().take(10).all(|c| c.is_ascii_hexdigit()) { self.scan_submit(); }
        }
    }
    pub fn scan_submit(&self) {
        let ui = self.ui();
        let text = ui.get_scan_input().to_string();
        if text.trim().is_empty() { return; }
        ui.set_scan_input("".into());
        let v = match self.st.borrow().store.verify_report_text(&text) { Ok(v) => v, Err(e) => { self.fail(e); return; } };
        let Some(v) = v else {
            ui.set_sc_kind(3);
            ui.set_sc_text("This code is not from a report of this lab. Do not accept the paper without checking it another way.".into());
            self.st.borrow().sc_rows.set_vec(vec![]);
            return;
        };
        let Ok(o) = self.st.borrow().store.order(v.order_id) else { return };
        let p = &o.patient;
        ui.set_sc_kind(if v.genuine { 1 } else { 2 });
        ui.set_sc_text(if v.genuine { format!("GENUINE: this printout of {} matches the report the doctor approved. Check the values on the paper against the list below.", v.patient_name) }
            else { format!("WARNING: this printout of {} does NOT match the lab's record. Do not accept it as genuine.", v.patient_name) }.into());
        ui.set_sc_name(v.patient_name.as_str().into());
        ui.set_sc_meta(format!("ID {}  ·  {} {} / {}  ·  report B{:05}  ·  {}", p.patient_code, p.age, p.age_type.to_lowercase(), p.gender, o.id, fmt::when_full(&o.created_at)).into());
        let contact: Vec<&str> = [p.phone.as_str(), p.address.as_str()].into_iter().filter(|x| !x.trim().is_empty()).collect();
        ui.set_sc_contact(contact.join("  ·  ").into());
        ui.set_sc_money(format!("BILL {} · DUE {}", fmt::rupees(o.total_paise), fmt::rupees(o.due_paise().max(0))).into());
        ui.set_sc_status(o.status.as_str().into());
        ui.set_sc_order(o.id as i32);
        ui.set_sc_patient(p.id as i32);
        let approved = Order { tests: o.tests.iter().filter(|t| t.is_approved()).cloned().collect(), ..o.clone() };
        let (rows, _) = sum_rows(&approved, false);
        self.st.borrow().sc_rows.set_vec(rows);
    }
}

// ---------- analyzer results: receiving, inbox, attaching ----------
fn hist_row(h: &labdesk_core::Histogram) -> HistRow {
    use labdesk_core::machine as m;
    let vis = m::visible_fraction(&h.name);
    let n = h.data.len().max(1);
    let upto = ((n as f32 * vis) as usize).clamp(1, n);
    let mut path = String::from("M 0 100");
    for (i, v) in h.data.iter().take(upto).enumerate() { path.push_str(&format!(" L {:.1} {:.1}", i as f32 * 256.0 / upto as f32, 100.0 - *v as f32 * 100.0 / 255.0)); }
    path.push_str(" L 256 100 Z");
    let lines: Vec<f32> = h.lines.iter().map(|c| (*c as f32 / (256.0 * vis)).clamp(0.0, 1.0)).collect();
    let labels: Vec<SharedString> = h.lines.iter().map(|c| m::line_label(&h.name, *c).to_string().into()).collect();
    let fill = match h.name.as_str() { "WBC" => slint::Color::from_argb_u8(0xd8, 0xf0, 0xea, 0xd0), "RBC" => slint::Color::from_argb_u8(0xe0, 0xe5, 0x41, 0x3a), _ => slint::Color::from_argb_u8(0xe0, 0x2e, 0xc2, 0x4a) };
    HistRow { name: match h.name.as_str() { "WBC" => "WBC".into(), "RBC" => "RBC".into(), _ => "PLT".into() }, path: path.into(), lines: ModelRc::from(Rc::new(VecModel::from(lines))), labels: ModelRc::from(Rc::new(VecModel::from(labels))), fill: fill.into() }
}

impl Ctl {
    /// Start listening for the analyzer if the Admin turned that on. Called when the program opens.
    fn machine_autostart(&self) {
        let (on, port) = self.st.borrow().store.machine_auto().unwrap_or((false, 5000));
        let ui = self.ui();
        ui.set_mc_port(port.to_string().into());
        ui.set_mc_range(if self.st.borrow().store.machine_range_mode().unwrap_or_default() == "labdesk" { 1 } else { 0 });
        if on { self.start_receiver(port, false); } else { ui.set_mc_auto_status("Receiving is off.".into()); }
    }
    fn start_receiver(&self, port: u16, announce: bool) -> bool {
        let ui = self.ui();
        let (tx, rx) = std::sync::mpsc::channel();
        match labdesk_core::capture::Capture::start_with(port, &self.opts.docs_dir.join("MachineMessages"), false, Some(tx)) {
            Ok(cap) => {
                *self.capture.borrow_mut() = Some(cap);
                *self.rx.borrow_mut() = Some(rx);
                ui.set_mc_auto(true);
                ui.set_mc_auto_status(format!("Receiving on port {port}. Waiting for the analyzer…").into());
                let me = self.me.borrow().clone();
                self.rx_timer.start(TimerMode::Repeated, Duration::from_millis(1000), move || { if let Some(c) = me.upgrade() { c.drain_machine(); } });
                if announce { self.toast("Receiving from the analyzer is on", false); }
                true
            }
            Err(e) => {
                ui.set_mc_auto(false);
                ui.set_mc_auto_status(friendly(&e).into());
                if announce { ui.set_err_mc(friendly(&e).into()); } else { self.toast("The analyzer port is busy. Close the analyzer's own software, then turn receiving on in Settings.", true); }
                false
            }
        }
    }
    fn mc_auto_toggle(&self) {
        let ui = self.ui();
        if ui.get_mc_auto() {
            let actor = self.actor();
            if self.st.borrow_mut().store.set_machine_auto(&actor, false, ui.get_mc_port().trim().parse().unwrap_or(5000)).is_err() { self.toast("Only an Admin can change this.", true); return; }
            *self.capture.borrow_mut() = None;
            *self.rx.borrow_mut() = None;
            self.rx_timer.stop();
            ui.set_mc_auto(false);
            ui.set_mc_auto_status("Receiving is off.".into());
            return;
        }
        let port: u16 = match ui.get_mc_port().trim().parse() { Ok(p) if p >= 1024 => p, _ => { ui.set_err_mc("Type the port number shown on the machine, for example 5000".into()); return; } };
        let actor = self.actor();
        if let Err(e) = self.st.borrow_mut().store.set_machine_auto(&actor, true, port) { self.fail(e); return; }
        if !self.start_receiver(port, true) { let _ = self.st.borrow_mut().store.set_machine_auto(&actor, false, port); }
    }
    fn mc_range_set(&self, i: i32) {
        let actor = self.actor();
        let mode = if i == 1 { "labdesk" } else { "analyzer" };
        let r = self.st.borrow_mut().store.set_machine_range_mode(&actor, mode);
        match r {
            Ok(()) => self.toast(if i == 1 { "Reports will use LabDesk's own ranges" } else { "Reports will use the same ranges as the analyzer" }, false),
            Err(e) => { self.fail(e); let cur = self.st.borrow().store.machine_range_mode().unwrap_or_default(); self.ui().set_mc_range(if cur == "labdesk" { 1 } else { 0 }); }
        }
    }
    /// Take what the receiver thread collected and put it in the inbox.
    fn drain_machine(&self) {
        let msgs: Vec<String> = { let rx = self.rx.borrow(); match rx.as_ref() { Some(r) => r.try_iter().collect(), None => vec![] } };
        let mut got = 0;
        for m in msgs {
            match self.st.borrow_mut().store.save_machine_message(&m) { Ok(Some(_)) => got += 1, Ok(None) => {}, Err(_) => {} }
        }
        let status = self.capture.borrow().as_ref().map(|k| { let (n, last) = k.status(); if n == 0 && last.is_empty() { format!("Receiving on port {}. Waiting for the analyzer…", k.port()) } else { format!("Receiving on port {}. {n} message(s) so far. {last}", k.port()) } });
        if let Some(t) = status { self.ui().set_mc_auto_status(t.into()); }
        if got > 0 {
            self.toast(&format!("{} new result{} from the analyzer", got, if got == 1 { "" } else { "s" }), false);
            if self.ui().get_page() == 8 { self.refresh_machine(); } else { self.refresh_machine_badge(); }
        }
    }
    fn refresh_machine_badge(&self) { let n = self.st.borrow().store.machine_new_count().unwrap_or(0); self.ui().set_mach_new(n as i32); }
    pub fn refresh_machine(&self) {
        let ui = self.ui();
        self.refresh_machine_badge();
        let list = self.st.borrow().store.machine_inbox(false, 200).unwrap_or_default();
        self.st.borrow().mach_list.set_vec(list.iter().map(|e| MachRow { id: e.id as i32, name: if e.patient_name.is_empty() { "(no name)".into() } else { SharedString::from(e.patient_name.as_str()) }, sample: e.sample_id.as_str().into(),
            time: e.test_time.as_str().into(), status: e.status.to_uppercase().into(), abn: if e.abnormal > 0 { format!("{} marked", e.abnormal) } else { "all in range".to_string() }.into() }).collect::<Vec<_>>());
        let sel = ui.get_mach_selected();
        if sel != 0 && list.iter().any(|e| e.id == sel as i64) { self.show_machine(sel as i64); } else if sel != 0 { ui.set_mach_selected(0); }
    }
    fn show_machine(&self, id: i64) {
        use labdesk_core::machine as m;
        let ui = self.ui();
        let (d, entry) = { let st = self.st.borrow(); (st.store.machine_data(id), st.store.machine_inbox(false, 200).unwrap_or_default().into_iter().find(|e| e.id == id)) };
        let (Ok(d), Some(e)) = (d, entry) else { ui.set_mach_selected(0); return };
        ui.set_mach_selected(id as i32);
        ui.set_md_name(if d.patient_name.is_empty() { "(no name typed on the analyzer)".to_string() } else { d.patient_name.clone() }.into());
        ui.set_md_meta(format!("Sample {}  ·  {}  ·  {}{}  ·  tested {}", d.sample_id, if d.sex == "F" { "Female" } else if d.sex == "M" { "Male" } else { d.sex.as_str() }, d.age_text, if d.age_text.is_empty() { "" } else { " years" }, d.test_time).into());
        ui.set_md_status(match e.status.as_str() { "New" => "Pending", "Attached" => "Completed", _ => "Delivered" }.into());
        ui.set_md_new(e.status == "New");
        ui.set_md_order(if e.status == "Attached" { e.order_id.unwrap_or(0) as i32 } else { 0 });
        ui.set_mach_can_approve(matches!(self.actor().role, Role::Admin | Role::Pathologist));
        if e.status == "New" {
            ui.set_qp_name(d.patient_name.as_str().into()); ui.set_qp_age(d.age_text.trim().trim_end_matches(|c: char| c.is_alphabetic()).into());
            ui.set_qp_gender(match d.sex.as_str() { "M" => 0, "F" => 1, _ => 2 }); ui.set_qp_phone("".into()); ui.set_err_qp("".into());
        }
        ui.set_md_note(match (e.status.as_str(), e.order_id) { ("Attached", Some(o)) => format!("Attached to report B{o:05}. The doctor can now check and approve it."), ("Dismissed", _) => "This result was dismissed.".into(), _ => String::new() }.into());
        let items: Vec<MachItemRow> = d.items.iter().filter(|it| m::ITEMS.iter().any(|i| i.code.eq_ignore_ascii_case(&it.code))).map(|it| {
            let item = m::ITEMS.iter().find(|i| i.code.eq_ignore_ascii_case(&it.code));
            MachItemRow { name: it.code.as_str().into(), value: m::shown_value(&it.code, &it.value).into(), unit: item.map_or(it.unit.as_str(), |i| i.unit).into(),
                range: item.map_or(it.range.clone(), m::analyzer_range).into(), flag: m::analyzer_flag(it).into() }
        }).collect();
        self.st.borrow().md_items.set_vec(items);
        self.st.borrow().md_hists.set_vec(d.histograms.iter().map(hist_row).collect::<Vec<_>>());
        let cands = if e.status == "New" { self.st.borrow().store.machine_candidates(id).unwrap_or_default() } else { vec![] };
        self.st.borrow().md_cands.set_vec(cands.iter().map(|c| CandRow { order_test_id: c.order_test_id as i32,
            title: format!("{}  ·  {}", c.patient_name, c.patient_code).into(), sub: format!("{}  ·  registered {}", c.test_name, fmt::when_full(&c.registered)).into(),
            hint: if c.score >= 100 { "NAME MATCHES" } else if c.score > 0 { "PARTLY MATCHES" } else { "" }.into() }).collect::<Vec<_>>());
    }
    fn machine_attach(&self, order_test_id: i64) {
        let id = self.ui().get_mach_selected() as i64;
        if id == 0 { return; }
        let actor = self.actor();
        let r = self.st.borrow_mut().store.attach_machine_result(&actor, id, order_test_id);
        match r {
            Ok(o) => { self.toast(&format!("Attached to report B{:05}. The doctor can now check and approve it.", o.id), false); self.refresh_machine(); self.refresh_orders(); self.refresh_home(); }
            Err(e) => self.fail(e),
        }
    }
    /// One step: patient + blood count report + attached values, then open the report so it can be edited.
    fn machine_quick(&self) {
        let ui = self.ui();
        let id = ui.get_mach_selected() as i64;
        if id == 0 { return; }
        let name = ui.get_qp_name().to_string();
        let words: Vec<&str> = name.split_whitespace().collect();
        let skip = words.first().is_some_and(|w| matches!(w.trim_end_matches('.').to_lowercase().as_str(), "mr" | "mrs" | "ms" | "miss" | "master" | "baby" | "dr"));
        let words: Vec<&str> = if skip && words.len() > 1 { words[1..].to_vec() } else { words };
        if words.is_empty() { ui.set_err_qp("Type the patient's name".into()); return; }
        let age: f64 = match ui.get_qp_age().trim().parse() { Ok(a) if a > 0.0 && a < 130.0 => a, _ => { ui.set_err_qp("Type the age in years".into()); return; } };
        let (first, last) = if words.len() == 1 { (words[0].to_string(), String::new()) } else { (words[..words.len() - 1].join(" "), words[words.len() - 1].to_string()) };
        let gender = ["Male", "Female", "Other"][ui.get_qp_gender().clamp(0, 2) as usize].to_string();
        let p = NewPatient { designation: String::new(), first_name: first, last_name: last, age, age_type: "Year".into(), gender, phone: ui.get_qp_phone().trim().to_string(), email: String::new(), address: String::new() };
        let actor = self.actor();
        let r = self.st.borrow_mut().store.report_from_machine(&actor, id, p);
        match r {
            Ok(o) => { self.toast(&format!("Report B{:05} made with the analyzer values. Check it, then approve.", o.id), false); self.refresh_machine(); self.refresh_orders(); self.refresh_home(); }
            Err(e) => self.fail(e),
        }
    }
    /// Open the report this result went into (and, if asked, start approval, then print the PDF).
    fn machine_open_report(&self, print: bool) {
        let oid = self.ui().get_md_order() as i64;
        if oid == 0 { return; }
        self.open_order(oid);
        if print { self.print_after.set(true); self.approve(); if !self.ui().get_ap_shown() { self.print_after.set(false); } }
    }
    fn machine_dismiss(&self) {
        let id = self.ui().get_mach_selected() as i64;
        let actor = self.actor();
        let r = self.st.borrow_mut().store.dismiss_machine_result(&actor, id);
        match r { Ok(()) => { self.toast("Dismissed", false); self.refresh_machine(); } Err(e) => self.fail(e) }
    }
}

impl Ctl {
    /// Used by the screenshot tool and tests: pretend the analyzer just sent this message.
    #[doc(hidden)]
    pub fn inject_machine_message(&self, raw: &str) -> bool { matches!(self.st.borrow_mut().store.save_machine_message(raw), Ok(Some(_))) }
}

// ---------- crash report window ----------
impl Ctl {
    /// After a crash, the next start shows what would be sent and lets the person decide.
    fn show_crash_if_any(&self) {
        if !diag::crashed_last_time(&self.opts.data_dir) { return; }
        let recent: Vec<String> = self.st.borrow().store.audit_log(30).unwrap_or_default().into_iter().map(|(t, who, what, ent, id)| format!("{t}  {who}  {what} {ent} {id}")).collect();
        let text = diag::crash_report(&self.opts.data_dir, &self.opts.db_path, &recent);
        *self.crash_text.borrow_mut() = text.clone();
        let ui = self.ui();
        ui.set_crash_text(text.into());
        ui.set_crash_shown(true);
    }
    fn crash_save_file(&self) -> Option<PathBuf> {
        let text = self.crash_text.borrow().clone();
        let dir = self.opts.docs_dir.join("Problem reports");
        let file = dir.join(format!("LabDesk-crash-report-{}.txt", chrono::Local::now().format("%Y-%m-%d_%H%M%S")));
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&file, text)) {
            Ok(()) => Some(file),
            Err(e) => { self.toast(&format!("Could not save the report: {e}"), true); None }
        }
    }
    fn crash_save(&self) {
        if let Some(f) = self.crash_save_file() {
            self.ui().set_crash_shown(false);
            self.open_path(&f);
            self.toast(&format!("Crash report saved in {}. Please send that file to your software provider.", self.opts.docs_dir.join("Problem reports").display()), false);
        }
    }
    fn crash_send(&self) {
        let Some(f) = self.crash_save_file() else { return };
        self.ui().set_crash_shown(false);
        self.open_url(&diag::report_mailto(&self.crash_text.borrow()));
        self.toast(&format!("Your email program is opening with the report. A copy is saved as {}", f.display()), false);
    }
}

// ---------- updates ----------
impl Ctl {
    /// Look for a newer version on a worker thread. `manual` also reports "up to date" and problems.
    fn update_check(&self, manual: bool) {
        if self.upd_rx.borrow().is_some() { return; }
        let ui = self.ui();
        ui.set_upd_busy(true);
        ui.set_upd_status("Looking for a newer version…".into());
        let (tx, rx) = std::sync::mpsc::channel();
        let data = self.opts.data_dir.clone();
        std::thread::spawn(move || { let _ = tx.send(updater::check_and_download(&data, updater::VERSION, updater::UPDATE_REPO)); });
        *self.upd_rx.borrow_mut() = Some(rx);
        let me = self.me.borrow().clone();
        self.upd_timer.start(TimerMode::Repeated, Duration::from_millis(700), move || { if let Some(c) = me.upgrade() { c.update_poll(manual); } });
    }
    fn update_poll(&self, manual: bool) {
        use std::sync::mpsc::TryRecvError;
        let got = match self.upd_rx.borrow().as_ref().map(|r| r.try_recv()) {
            Some(Ok(o)) => Some(o),
            Some(Err(TryRecvError::Disconnected)) => Some(updater::Outcome::Failed("The update check stopped unexpectedly.".into())),
            _ => None,
        };
        let Some(o) = got else { return };
        self.upd_timer.stop();
        *self.upd_rx.borrow_mut() = None;
        let ui = self.ui();
        ui.set_upd_busy(false);
        match o {
            updater::Outcome::UpToDate => { ui.set_upd_status(format!("LabDesk {} is the newest version.", updater::VERSION).into()); if manual { self.toast("LabDesk is up to date.", false); } }
            updater::Outcome::Ready(v) => {
                ui.set_upd_ready(true);
                ui.set_upd_status(format!("Version {v} is downloaded and checked. It installs the next time LabDesk starts, or press INSTALL NOW (finish anything you are typing first).").into());
                self.toast(&format!("LabDesk {v} is ready. It installs the next time you open LabDesk (or press INSTALL NOW in Settings > About)."), false);
            }
            updater::Outcome::Failed(e) => { ui.set_upd_status(e.clone().into()); if manual { self.toast(&e, true); } }
        }
    }
    /// Back up, start the installer and close this window. The installer opens LabDesk again when it is done.
    fn update_install(&self) {
        let Some(p) = updater::read_pending(&self.opts.data_dir, updater::VERSION) else {
            self.ui().set_upd_ready(false);
            self.toast("No update is waiting. Press CHECK FOR UPDATES.", true);
            return;
        };
        let _ = self.st.borrow_mut().store.backup_now(&Actor::new("LabDesk (before update)", Role::Admin));
        match updater::launch_installer(&self.opts.data_dir, &p, true) {
            Ok(()) => { let _ = slint::quit_event_loop(); }
            Err(e) => self.toast(&e, true),
        }
    }
    /// At start: show the update state and, once a day, look for a new version quietly.
    fn start_updates(&self) {
        let ui = self.ui();
        let data = &self.opts.data_dir;
        ui.set_upd_auto(updater::auto_enabled(data));
        ui.set_upd_ready(updater::read_pending(data, updater::VERSION).is_some());
        ui.set_upd_status(format!("You have LabDesk {}.", updater::VERSION).into());
        // look for a new version every time LabDesk starts, and again every 6 hours if it stays open
        if self.opts.use_sessions && updater::auto_enabled(data) {
            self.update_check(false);
            let me = self.me.borrow().clone();
            self.upd_again.start(TimerMode::Repeated, Duration::from_secs(6 * 3600), move || { if let Some(c) = me.upgrade() { if updater::auto_enabled(&c.opts.data_dir) { c.update_check(false); } } });
        }
    }
    /// The person closes LabDesk: a downloaded update installs quietly now, so the next start is already the new
    /// version. (A backup is made first. LabDesk is not opened again; the person does that.)
    fn before_exit(&self) {
        if !self.opts.use_sessions { return; }
        let Some(p) = updater::read_pending(&self.opts.data_dir, updater::VERSION) else { return };
        let _ = self.st.borrow_mut().store.backup_now(&Actor::new("LabDesk (before update)", Role::Admin));
        let _ = updater::launch_installer(&self.opts.data_dir, &p, false);
    }
    /// Move the window with the mouse. A maximized window is first restored under the pointer, like other programs do.
    fn window_drag(&self, x: f32, y: f32) {
        let ui = self.ui();
        let w = ui.window();
        let (sx, sy) = self.win_drag.get();
        if (x - sx).abs() + (y - sy).abs() < 3.0 { return; }
        let scale = w.scale_factor();
        let pos = w.position();
        if w.is_maximized() {
            let cur_w = (w.size().width as f32 / scale).max(1.0);
            let frac = (sx / cur_w).clamp(0.0, 1.0);
            let (nw, _) = self.win_normal.get();
            w.set_maximized(false);
            ui.set_win_maxed(false);
            let nx = pos.x as f32 + x * scale - frac * nw * scale;
            let ny = pos.y as f32 + (y - sy) * scale;
            w.set_position(slint::PhysicalPosition::new(nx as i32, ny as i32));
            self.win_drag.set((frac * nw, sy));
        } else {
            w.set_position(slint::PhysicalPosition::new(pos.x + ((x - sx) * scale) as i32, pos.y + ((y - sy) * scale) as i32));
        }
    }
}
