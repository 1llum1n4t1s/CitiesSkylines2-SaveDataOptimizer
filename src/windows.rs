use cok_inspector::save_data::{CleanupPlan, clean_save, preview_cleanup};
use std::ffi::{OsStr, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::ptr;

type Handle = isize;
type Wparam = usize;
type Lparam = isize;
type ResultCode = isize;
const WM_DESTROY: u32 = 0x0002;
const WM_SIZE: u32 = 0x0005;
const WM_COMMAND: u32 = 0x0111;
const WM_CTLCOLORSTATIC: u32 = 0x0138;
const WM_GETMINMAXINFO: u32 = 0x0024;
const WM_SETFONT: u32 = 0x0030;
const GWLP_USERDATA: i32 = -21;
const BUTTON_ID: usize = 101;
const TRACE_ID: usize = 102;
const GAME_ID: usize = 104;
const CLEANUP_ID: usize = 103;

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}
#[repr(C)]
struct MinMaxInfo {
    reserved: Point,
    max_size: Point,
    max_position: Point,
    min_track: Point,
    max_track: Point,
}
#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}
#[repr(C)]
struct Msg {
    hwnd: Handle,
    message: u32,
    wparam: Wparam,
    lparam: Lparam,
    time: u32,
    pt: Point,
    private: u32,
}
#[repr(C)]
struct WndClass {
    style: u32,
    proc: Option<unsafe extern "system" fn(Handle, u32, Wparam, Lparam) -> ResultCode>,
    class_extra: i32,
    window_extra: i32,
    instance: Handle,
    icon: Handle,
    cursor: Handle,
    background: Handle,
    menu_name: *const u16,
    class_name: *const u16,
}
#[repr(C)]
struct OpenFileName {
    size: u32,
    owner: Handle,
    instance: Handle,
    filter: *const u16,
    custom_filter: *mut u16,
    max_custom_filter: u32,
    filter_index: u32,
    file: *mut u16,
    max_file: u32,
    file_title: *mut u16,
    max_file_title: u32,
    initial_dir: *const u16,
    title: *const u16,
    flags: u32,
    file_offset: u16,
    file_extension: u16,
    default_ext: *const u16,
    custom_data: isize,
    hook: *const c_void,
    template_name: *const u16,
    reserved: *mut c_void,
    reserved_size: u32,
    flags_ex: u32,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterClassW(class: *const WndClass) -> u16;
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: Handle,
        menu: Handle,
        instance: Handle,
        param: *mut c_void,
    ) -> Handle;
    fn DefWindowProcW(hwnd: Handle, msg: u32, w: Wparam, l: Lparam) -> ResultCode;
    fn SetWindowLongPtrW(hwnd: Handle, index: i32, value: isize) -> isize;
    fn GetWindowLongPtrW(hwnd: Handle, index: i32) -> isize;
    fn ShowWindow(hwnd: Handle, command: i32) -> i32;
    fn EnableWindow(hwnd: Handle, enable: i32) -> i32;
    fn GetSysColorBrush(index: i32) -> Handle;
    fn UpdateWindow(hwnd: Handle) -> i32;
    fn GetMessageW(msg: *mut Msg, hwnd: Handle, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> ResultCode;
    fn PostQuitMessage(code: i32);
    fn SendMessageW(hwnd: Handle, msg: u32, w: Wparam, l: Lparam) -> ResultCode;
    fn SetWindowTextW(hwnd: Handle, text: *const u16) -> i32;
    fn MoveWindow(hwnd: Handle, x: i32, y: i32, w: i32, h: i32, repaint: i32) -> i32;
    fn GetClientRect(hwnd: Handle, rect: *mut Rect) -> i32;
    fn LoadCursorW(instance: Handle, name: *const u16) -> Handle;
    fn MessageBoxW(hwnd: Handle, text: *const u16, title: *const u16, kind: u32) -> i32;
    fn SetProcessDPIAware() -> i32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Handle;
}
#[link(name = "gdi32")]
unsafe extern "system" {
    fn SetBkMode(dc: Handle, mode: i32) -> i32;
    fn SetTextColor(dc: Handle, color: u32) -> u32;
    fn GetStockObject(index: i32) -> Handle;
    fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strikeout: u32,
        charset: u32,
        out_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch_family: u32,
        face: *const u16,
    ) -> Handle;
    fn DeleteObject(object: Handle) -> i32;
}
#[link(name = "comdlg32")]
unsafe extern "system" {
    fn GetOpenFileNameW(ofn: *mut OpenFileName) -> i32;
    fn GetSaveFileNameW(ofn: *mut OpenFileName) -> i32;
}

#[link(name = "bcrypt")]
unsafe extern "system" {
    fn BCryptGenRandom(algorithm: Handle, buffer: *mut u8, length: u32, flags: u32) -> i32;
}

fn new_asset_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    if unsafe { BCryptGenRandom(0, bytes.as_mut_ptr(), 16, 2) } != 0 {
        return Err("アセットIDを生成できません".into());
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain(Some(0)).collect()
}

struct App {
    hwnd: Handle,
    title: Handle,
    open_button: Handle,
    source: Handle,
    group_titles: [Handle; 2],
    metrics: [Handle; 5],
    metrics_labels: [Handle; 5],
    setup_title: Handle,
    trace_button: Handle,
    game_button: Handle,
    save_button: Handle,
    status: Handle,
    input: Option<PathBuf>,
    plan: Option<CleanupPlan>,
    trace: Option<PathBuf>,
    plugin: Option<PathBuf>,
}

fn label(control: Handle, text: impl AsRef<OsStr>) {
    unsafe {
        SetWindowTextW(control, wide(text).as_ptr());
    }
}

fn number(value: u64) -> String {
    let text = value.to_string();
    let first = (text.len() - 1) % 3 + 1;
    let (head, tail) = text.split_at(first);
    let mut result = head.to_owned();
    for group in tail.as_bytes().chunks_exact(3) {
        result.push(',');
        result.push_str(std::str::from_utf8(group).unwrap());
    }
    result
}

fn stride_path(path: &std::path::Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".stride");
    PathBuf::from(name)
}

impl App {
    fn layout(&self) {
        let mut rect: Rect = unsafe { std::mem::zeroed() };
        unsafe {
            GetClientRect(self.hwnd, &mut rect);
        }
        let width = rect.right.max(650);
        let height = rect.bottom.max(455);
        let place = |handle, x, y, w, h| unsafe {
            MoveWindow(handle, x, y, w, h, 1);
        };
        let margin = 32;
        place(self.title, margin, 26, width - 230, 43);
        place(self.open_button, width - 178, 32, 146, 34);
        place(self.source, margin, 82, width - 64, 30);

        let gap = 24;
        let card = (width - 64 - gap) / 2;
        let right = margin + card + gap;
        for (group, x) in [margin, right].into_iter().enumerate() {
            place(self.group_titles[group], x, 131, card, 28);
        }
        for (index, (x, columns, column)) in [
            (margin, 2, 0),
            (margin, 2, 1),
            (right, 3, 0),
            (right, 3, 1),
            (right, 3, 2),
        ]
        .into_iter()
        .enumerate()
        {
            let col = card / columns;
            let x = x + column * col;
            place(self.metrics_labels[index], x, 175, col, 27);
            place(self.metrics[index], x, 204, col, 52);
        }

        place(self.setup_title, margin, 286, width - 64, 27);
        place(self.trace_button, margin, 327, card, 39);
        place(self.game_button, right, 327, card, 39);
        place(self.status, margin, height - 64, width - 255, 56);
        place(self.save_button, width - 202, height - 62, 170, 38);
    }

    fn refresh(&self) {
        let values = self.plan.as_ref().map(|p| {
            [
                p.vehicle_ids.len() as u64,
                p.pedestrians,
                p.citizens,
                p.households,
                p.parked,
            ]
        });
        for (index, control) in self.metrics.iter().enumerate() {
            label(
                *control,
                values.map_or_else(|| "—".into(), |counts| number(counts[index])),
            );
        }
        label(
            self.trace_button,
            if self.trace.is_some() {
                "読み込み記録 ✓　変更…"
            } else {
                "読み込み記録を選ぶ…"
            },
        );
        label(
            self.game_button,
            if self.plugin.is_some() {
                "ゲームの部品 ✓　変更…"
            } else {
                "ゲームの部品を選ぶ…"
            },
        );
        let has_changes = self
            .plan
            .as_ref()
            .is_some_and(|p| !p.vehicle_ids.is_empty() || p.pedestrians != 0);
        let (ready, status) = match (
            self.plan.is_some(),
            has_changes,
            self.trace.is_some(),
            self.plugin.is_some(),
        ) {
            (false, _, _, _) => (false, "まず、右上の「セーブを開く」を押してください。"),
            (_, false, _, _) => (
                false,
                "このセーブに消せる車や歩行者は見つかりませんでした。",
            ),
            (_, _, false, _) => (
                false,
                "このセーブをゲームで開いたときの読み込み記録を選んでください。",
            ),
            (_, _, _, false) => (
                false,
                "ゲームのフォルダーにある PipelinePlugin.dll を選んでください。",
            ),
            _ => (true, "準備ができました。新しい名前でコピーを作れます。"),
        };
        label(self.status, status);
        unsafe {
            EnableWindow(self.trace_button, self.plan.is_some() as i32);
            EnableWindow(self.game_button, self.plan.is_some() as i32);
            EnableWindow(self.save_button, ready as i32);
        }
    }

    fn open(&mut self, path: PathBuf) {
        label(self.status, "セーブを調べています…");
        unsafe {
            UpdateWindow(self.status);
        }
        match preview_cleanup(&path) {
            Ok(plan) => {
                let filename = path.file_name().unwrap_or(path.as_os_str());
                label(
                    self.source,
                    format!("セーブ　{}", filename.to_string_lossy()),
                );
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                // 同じ名前の記録と、その記録に対応する補助ファイルが揃っている場合だけ選ぶ。
                self.trace = [format!("{stem}.bin"), format!("snapshot-{stem}.bin")]
                    .into_iter()
                    .map(|name| path.with_file_name(name))
                    .find(|candidate| candidate.is_file() && stride_path(candidate).is_file());
                self.input = Some(path);
                self.plan = Some(plan);
                self.refresh();
            }
            Err(error) => {
                self.refresh();
                self.error("このセーブを開けませんでした", &error);
            }
        }
    }

    fn error(&self, title: &str, detail: &str) {
        unsafe {
            MessageBoxW(self.hwnd, wide(detail).as_ptr(), wide(title).as_ptr(), 0x10);
        }
    }

    fn picker(&mut self) {
        if let Some(path) = self.choose("セーブファイル (*.cok)\0*.cok\0\0", "セーブを開く", false)
        {
            self.open(path);
        }
    }

    fn pick_trace(&mut self) {
        let Some(path) = self.choose(
            "読み込み記録 (*.bin)\0*.bin\0\0",
            "このセーブの読み込み記録を選ぶ",
            false,
        ) else {
            return;
        };
        if !stride_path(&path).is_file() {
            self.error(
                "読み込み記録が足りません",
                "選んだファイルと同じ場所に、末尾に .stride が付くファイルも置いてください。",
            );
            return;
        }
        self.trace = Some(path);
        self.refresh();
    }

    fn pick_game(&mut self) {
        let Some(path) = self.choose(
            "ゲームの部品\0PipelinePlugin.dll\0\0",
            "PipelinePlugin.dll を選ぶ",
            false,
        ) else {
            return;
        };
        if !path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case("PipelinePlugin.dll"))
        {
            self.error(
                "ゲームの部品を選び直してください",
                "選ぶファイルの名前は PipelinePlugin.dll です。",
            );
            return;
        }
        self.plugin = Some(path);
        self.refresh();
    }

    fn choose(&self, filter: &str, title: &str, output: bool) -> Option<PathBuf> {
        let mut buffer = vec![0u16; 32768];
        let filter: Vec<u16> = filter.encode_utf16().collect();
        let title = wide(title);
        let extension = wide("cok");
        let mut ofn: OpenFileName = unsafe { std::mem::zeroed() };
        ofn.size = std::mem::size_of::<OpenFileName>() as u32;
        ofn.owner = self.hwnd;
        ofn.filter = filter.as_ptr();
        ofn.file = buffer.as_mut_ptr();
        ofn.max_file = buffer.len() as u32;
        ofn.title = title.as_ptr();
        ofn.flags = 0x00000800 | 0x00080000 | 0x00000008;
        let result = if output {
            ofn.default_ext = extension.as_ptr();
            unsafe { GetSaveFileNameW(&mut ofn) }
        } else {
            ofn.flags |= 0x00001000;
            unsafe { GetOpenFileNameW(&mut ofn) }
        };
        if result != 0 {
            let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
            return Some(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..len])));
        }
        None
    }

    fn save(&self) -> Result<(), String> {
        let (Some(input), Some(plan), Some(trace), Some(plugin)) =
            (&self.input, &self.plan, &self.trace, &self.plugin)
        else {
            return Err("セーブと、保存に必要なファイルを選んでください。".into());
        };
        let Some(output) = self.choose(
            "セーブファイル (*.cok)\0*.cok\0\0",
            "整理したセーブの名前を決める",
            true,
        ) else {
            return Ok(());
        };
        let ids = [
            new_asset_id()?,
            new_asset_id()?,
            new_asset_id()?,
            new_asset_id()?,
        ];
        label(self.status, "新しいセーブを作っています…");
        unsafe {
            UpdateWindow(self.status);
        }
        clean_save(
            input,
            &output,
            trace,
            &stride_path(trace),
            plugin,
            ids.each_ref().map(String::as_str),
            Some(plan),
        )?;
        label(
            self.status,
            format!(
                "保存しました　{}",
                output
                    .file_name()
                    .unwrap_or(output.as_os_str())
                    .to_string_lossy()
            ),
        );
        Ok(())
    }
}

unsafe extern "system" fn window_proc(hwnd: Handle, msg: u32, w: Wparam, l: Lparam) -> ResultCode {
    let app = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App };
    match msg {
        WM_SIZE if !app.is_null() => {
            unsafe {
                (*app).layout();
            }
            0
        }
        WM_GETMINMAXINFO => {
            let info = unsafe { &mut *(l as *mut MinMaxInfo) };
            info.min_track = Point { x: 700, y: 530 };
            0
        }
        WM_CTLCOLORSTATIC => unsafe {
            SetBkMode(w as Handle, 1);
            SetTextColor(w as Handle, 0x00463822);
            GetSysColorBrush(5)
        },
        WM_COMMAND if !app.is_null() && (w >> 16) & 0xffff == 0 => {
            let app = unsafe { &mut *app };
            match w & 0xffff {
                BUTTON_ID => app.picker(),
                TRACE_ID => app.pick_trace(),
                GAME_ID => app.pick_game(),
                CLEANUP_ID => {
                    if let Err(error) = app.save() {
                        app.refresh();
                        app.error("保存できませんでした", &error);
                    }
                }
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            unsafe {
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, w, l) },
    }
}

pub fn run(initial: Option<PathBuf>) {
    unsafe {
        SetProcessDPIAware();
        let instance = GetModuleHandleW(ptr::null());
        let class = wide("CokInspectorWindow");
        let wc = WndClass {
            style: 0,
            proc: Some(window_proc),
            class_extra: 0,
            window_extra: 0,
            instance,
            icon: 0,
            cursor: LoadCursorW(0, 32512usize as *const u16),
            background: 6,
            menu_name: ptr::null(),
            class_name: class.as_ptr(),
        };
        if RegisterClassW(&wc) == 0 {
            return;
        }
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide("Cities: Skylines II セーブデータ整理").as_ptr(),
            0x00CF0000,
            0x80000000u32 as i32,
            0x80000000u32 as i32,
            1000,
            570,
            0,
            0,
            instance,
            ptr::null_mut(),
        );
        if hwnd == 0 {
            return;
        }
        let child = |class: &str, text: &str, style: u32, id: usize| {
            CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(text).as_ptr(),
                0x50000000 | style,
                0,
                0,
                0,
                0,
                hwnd,
                id as isize,
                instance,
                ptr::null_mut(),
            )
        };
        let mut app = Box::new(App {
            hwnd,
            title: child("STATIC", "セーブデータ整理", 0, 0),
            open_button: child("BUTTON", "セーブを開く…", 0x10000, BUTTON_ID),
            source: child("STATIC", "セーブが選ばれていません", 0x4000, 0),
            group_titles: ["消すもの", "残すもの"].map(|text| child("STATIC", text, 0, 0)),
            metrics: ["—"; 5].map(|text| child("STATIC", text, 0, 0)),
            metrics_labels: [
                "車（台）",
                "歩行者（人）",
                "市民（人）",
                "世帯",
                "駐車中の車（台）",
            ]
            .map(|text| child("STATIC", text, 0, 0)),
            setup_title: child("STATIC", "保存の準備", 0, 0),
            trace_button: child("BUTTON", "", 0x10000, TRACE_ID),
            game_button: child("BUTTON", "", 0x10000, GAME_ID),
            save_button: child("BUTTON", "別名で保存…", 0x10001, CLEANUP_ID),
            status: child("STATIC", "", 0, 0),
            input: None,
            plan: None,
            trace: None,
            plugin: None,
        });
        let font = |height, weight| {
            CreateFontW(
                height,
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                1,
                0,
                0,
                0,
                0,
                wide("Yu Gothic UI").as_ptr(),
            )
        };
        let regular = font(-17, 400);
        let heading = font(-23, 650);
        let large = font(-33, 700);
        let fallback = GetStockObject(17);
        for (controls, face) in [
            (
                &[
                    app.source,
                    app.open_button,
                    app.trace_button,
                    app.game_button,
                    app.save_button,
                    app.status,
                ][..],
                regular,
            ),
            (&app.metrics_labels[..], regular),
            (
                &[app.group_titles[0], app.group_titles[1], app.setup_title][..],
                heading,
            ),
            (&[app.title][..], large),
            (&app.metrics[..], large),
        ] {
            for &control in controls {
                SendMessageW(
                    control,
                    WM_SETFONT,
                    if face != 0 { face } else { fallback } as usize,
                    1,
                );
            }
        }
        app.refresh();
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&mut *app as *mut App) as isize);
        app.layout();
        ShowWindow(hwnd, 1);
        UpdateWindow(hwnd);
        if let Some(path) = initial {
            app.open(path);
        }
        let mut msg: Msg = std::mem::zeroed();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        for face in [regular, heading, large] {
            if face != 0 {
                DeleteObject(face);
            }
        }
    }
}
