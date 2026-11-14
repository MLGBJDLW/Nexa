//! Observe the native menu loop of the selected window. A shared PID alone
//! cannot distinguish two independent windows in the same application.
use super::*;
use crate::tools::computer_use_tool::{MenuImage, MenuTarget, NativeMenuItem};
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, POINT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumThreadWindows, GetGUIThreadInfo, GetMenuBarInfo, GetMenuItemCount, GetMenuItemInfoW,
    GetMenuItemRect, GetWindowLongPtrW, MenuItemFromPoint, GUITHREADINFO, GWL_STYLE, HMENU,
    MENUBARINFO, MENUITEMINFOW, MFS_DISABLED, MFT_SEPARATOR, MIIM_FTYPE, MIIM_ID, MIIM_STATE,
    MIIM_STRING, MIIM_SUBMENU, OBJID_CLIENT, WS_POPUP,
};

fn menu_thread(parent: &WindowSnapshot) -> Option<u32> {
    let owner = hwnd(parent.id);
    let thread = unsafe { GetWindowThreadProcessId(owner, None) };
    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    if thread == 0 || unsafe { GetGUIThreadInfo(thread, &mut info) }.is_err() {
        return None;
    }
    (info.hwndMenuOwner == owner || unsafe { GetAncestor(info.hwndMenuOwner, GA_ROOT) } == owner)
        .then_some(thread)
}

fn menu_handle(window: HWND) -> Option<HMENU> {
    let mut info = MENUBARINFO {
        cbSize: size_of::<MENUBARINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetMenuBarInfo(window, OBJID_CLIENT, 0, &mut info) }.ok()?;
    (!info.hMenu.0.is_null()).then_some(info.hMenu)
}

fn owned_popup(window: HWND, parent: HWND) -> bool {
    let mut current = window;
    for _ in 0..16 {
        let Ok(owner) = (unsafe { GetWindow(current, GW_OWNER) }) else {
            return false;
        };
        if owner == parent {
            return true;
        }
        if owner == current || owner.0.is_null() {
            return false;
        }
        current = owner;
    }
    false
}

fn surfaces(parent: &WindowSnapshot) -> Vec<HWND> {
    struct Scan {
        parent: HWND,
        active_menu: bool,
        windows: Vec<HWND>,
    }
    unsafe extern "system" fn collect(window: HWND, data: LPARAM) -> BOOL {
        let scan = unsafe { &mut *(data.0 as *mut Scan) };
        if window != scan.parent && unsafe { IsWindowVisible(window) }.as_bool() {
            let native = window_class(window).is_ok_and(|class| class == "#32768");
            if (native && scan.active_menu)
                || (!native
                    && unsafe { GetWindowLongPtrW(window, GWL_STYLE) } & WS_POPUP.0 as isize != 0
                    && owned_popup(window, scan.parent))
            {
                scan.windows.push(window);
            }
        }
        BOOL(1)
    }
    let thread = unsafe { GetWindowThreadProcessId(hwnd(parent.id), None) };
    if thread == 0 {
        return Vec::new();
    }
    let mut scan = Scan {
        parent: hwnd(parent.id),
        active_menu: menu_thread(parent) == Some(thread),
        windows: Vec::new(),
    };
    let _ = unsafe {
        EnumThreadWindows(
            thread,
            Some(collect),
            LPARAM(std::ptr::addr_of_mut!(scan) as isize),
        )
    };
    scan.windows
}

// A window can own dialogs, tooltips and unrelated tool palettes. Only collect
// pixels for an owned custom popup when its UIA root or direct child is a menu.
fn is_menu_provider(handle: HWND) -> Result<bool, CoreError> {
    use windows::Win32::UI::Accessibility::UIA_MenuControlTypeId;
    let _apartment = ComApartment::initialize()?;
    let automation = create_automation()?;
    let root = unsafe { automation.ElementFromHandle(handle) }
        .map_err(|e| platform_error("identify popup root", e))?;
    if unsafe { root.CurrentControlType() }.ok() == Some(UIA_MenuControlTypeId) {
        return Ok(true);
    }
    let walker = unsafe { automation.ControlViewWalker() }
        .map_err(|e| platform_error("inspect popup root", e))?;
    // Only the first content child of a popup wrapper may establish menu scope;
    // a menu bar somewhere in an owned application window is insufficient.
    if let Ok(child) = unsafe { walker.GetFirstChildElement(&root) } {
        return Ok(unsafe { child.CurrentControlType() }.ok() == Some(UIA_MenuControlTypeId));
    }
    Ok(false)
}

fn verify_surface(
    parent: &WindowSnapshot,
    surface: &WindowSnapshot,
    menu: Option<HMENU>,
) -> Result<(), CoreError> {
    let window = hwnd(surface.id);
    let thread = unsafe { GetWindowThreadProcessId(window, None) };
    let linked = match menu {
        Some(menu) => {
            surface.window_class == "#32768"
                && menu_thread(parent) == Some(thread)
                && menu_handle(window) == Some(menu)
        }
        None => owned_popup(window, hwnd(parent.id)) && is_menu_provider(window)?,
    };
    if surface.pid != parent.pid || !linked {
        return Err(invalid(
            "The popup no longer belongs to the observed parent's menu loop.",
        ));
    }
    let current = current_window_for_access(surface, true)?.1;
    capture::ensure_visible(&current)?;
    if !same_capture_surface(surface, &current) || surface.input_epoch != current.input_epoch {
        return Err(invalid(
            "The observed popup moved or received user input. Observe again.",
        ));
    }
    Ok(())
}

fn item(
    menu: HMENU,
    position: u32,
    surface: &WindowSnapshot,
    width: u32,
    height: u32,
) -> Result<Option<UiElementSnapshot>, CoreError> {
    let mut name = [0u16; 512];
    let mut info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_FTYPE | MIIM_ID | MIIM_STATE | MIIM_STRING | MIIM_SUBMENU,
        dwTypeData: PWSTR(name.as_mut_ptr()),
        cch: name.len() as u32,
        ..Default::default()
    };
    unsafe { GetMenuItemInfoW(menu, position, true, &mut info) }
        .map_err(|e| platform_error("read observed menu item", e))?;
    if info.fType.contains(MFT_SEPARATOR) {
        return Ok(None);
    }
    let mut rect = RECT::default();
    unsafe { GetMenuItemRect(None, menu, position, &mut rect) }
        .map_err(|e| platform_error("locate observed menu item", e))?;
    let Some((bounds, screen_bounds)) = clipped_element_bounds(rect, surface, width, height) else {
        return Ok(None);
    };
    let enabled = info.fState.0 & MFS_DISABLED.0 == 0;
    let name = String::from_utf16_lossy(&name[..(info.cch as usize).min(name.len())]);
    Ok(Some(UiElementSnapshot {
        id: String::new(),
        role: "menu_item".into(),
        name,
        automation_id: String::new(),
        bounds,
        screen_bounds,
        enabled,
        focused: false,
        keyboard_focusable: false,
        interactive: enabled,
        password: false,
        actions: if enabled {
            vec!["click".into()]
        } else {
            vec![]
        },
        state: UiElementState::default(),
        menu_image: None,
        menu_target: Some(MenuTarget {
            surface: surface.clone(),
            native: Some(NativeMenuItem {
                handle: menu.0 as usize as u64,
                position,
                command: info.wID,
                submenu: info.hSubMenu.0 as usize as u64,
            }),
        }),
    }))
}

pub(super) fn observe(
    parent: &mut CapturedWindow,
    options: CaptureOptions,
) -> Result<(), CoreError> {
    let _physical = PhysicalPixels::enter()?;
    let handles = surfaces(&parent.snapshot);
    if handles.is_empty() {
        return Ok(());
    }
    let mut menu_items = Vec::new();
    let mut images = Vec::new();
    let mut captured_surfaces = Vec::new();
    const MAX_MENU_IMAGES: usize = 4;
    let mut truncated = false;
    for &handle in &handles {
        let native = window_class(handle)? == "#32768";
        let menu = if native {
            Some(menu_handle(handle).ok_or_else(|| invalid("Menu closed before capture."))?)
        } else if is_menu_provider(handle).unwrap_or(false) {
            None
        } else {
            continue;
        };
        if images.len() == MAX_MENU_IMAGES {
            truncated = true;
            break;
        }
        let window = Window::from_raw_hwnd(handle.0);
        let surface = snapshot(&window)?;
        verify_surface(&parent.snapshot, &surface, menu)?;
        let remaining = options.max_elements - menu_items.len();
        let start = menu_items.len();
        let (png, width, height, items) = if let Some(menu) = menu {
            let (png, width, height, _, _) = resized_png(capture_rgba(window, &surface)?.image)?;
            let count = unsafe { GetMenuItemCount(Some(menu)) };
            if count < 0 {
                return Err(invalid("Menu closed while collecting items."));
            }
            truncated |= count as usize > remaining;
            let mut items = Vec::new();
            for position in 0..(count as usize).min(remaining) {
                if let Some(element) = item(menu, position as u32, &surface, width, height)? {
                    items.push(element);
                }
            }
            (png, width, height, items)
        } else {
            let frame = capture_window_with_elements(
                &surface,
                CaptureOptions {
                    mode: CaptureMode::Raw,
                    text_context: None,
                    ..options
                },
                collect_elements,
            )?;
            truncated |= frame.semantic_truncated || frame.semantic_error.is_some();
            let mut items: Vec<_> = frame
                .elements
                .into_iter()
                .filter(|e| e.role == "menu_item")
                .collect();
            truncated |= items.len() > remaining;
            items.truncate(remaining);
            for element in &mut items {
                element.actions.retain(|s| s == "click");
                element.menu_target = Some(MenuTarget {
                    surface: surface.clone(),
                    native: None,
                });
            }
            (frame.png, frame.image_width, frame.image_height, items)
        };
        verify_surface(&parent.snapshot, &surface, menu)?;
        for mut element in items {
            element.id = format!("e{}", super::super::MAX_ELEMENTS + menu_items.len() + 1);
            element.menu_image = Some(images.len() + 1);
            menu_items.push(element);
        }
        let png = if options.mode == CaptureMode::SetOfMarks {
            annotate_elements(&png, &menu_items[start..])?
        } else {
            png
        };
        images.push(MenuImage {
            origin: ElementBounds {
                x: surface.x,
                y: surface.y,
                width: surface.width,
                height: surface.height,
            },
            png,
            width,
            height,
        });
        captured_surfaces.push((surface, menu));
    }
    let current = current_window_for_access(&parent.snapshot, true)?.1;
    if !same_capture_surface(&parent.snapshot, &current)
        || current.input_epoch != parent.snapshot.input_epoch
        || surfaces(&current) != handles
    {
        return Err(invalid(
            "The parent or its menu stack changed during observation. Observe again.",
        ));
    }
    for (surface, menu) in &captured_surfaces {
        verify_surface(&current, surface, *menu)?;
    }
    // Some UIA providers also project a popup's items into the owner's tree.
    // Keep the projection bound to the actual menu image and its own hit test.
    parent.elements.retain(|e| {
        e.role != "menu_item"
            || !menu_items.iter().any(|item| {
                e.name == item.name && {
                    let x = e.screen_bounds.x + (e.screen_bounds.width / 2) as i32;
                    let y = e.screen_bounds.y + (e.screen_bounds.height / 2) as i32;
                    let b = item.screen_bounds;
                    x >= b.x && x.abs_diff(b.x) < b.width && y >= b.y && y.abs_diff(b.y) < b.height
                }
            })
    });
    let remaining = options.max_elements - menu_items.len();
    parent.semantic_truncated |= truncated || parent.elements.len() > remaining;
    parent.elements.truncate(remaining);
    if options.mode == CaptureMode::SetOfMarks {
        parent.annotated_png = Some(annotate_elements(&parent.png, &parent.elements)?);
    }
    parent.elements.extend(menu_items);
    parent.menu_images = images;
    Ok(())
}

pub(super) fn click(
    parent: &WindowSnapshot,
    expected: &UiElementSnapshot,
    tracker: &ControlCommitTracker,
) -> Result<(), ControlFailure> {
    let target = expected.menu_target.as_ref().expect("observed menu");
    let menu = target
        .native
        .as_ref()
        .map(|n| HMENU(n.handle as usize as *mut c_void));
    let surface = &target.surface;
    tracker.result_as(
        PreCommitFailureKind::UserTakeover,
        ensure_target_foreground(parent),
    )?;
    tracker.result_as(
        PreCommitFailureKind::ObservationStale,
        verify_surface(parent, surface, menu),
    )?;
    let point = if let (Some(menu), Some(native)) = (menu, target.native.as_ref()) {
        let live = tracker.result(item(
            menu,
            native.position,
            surface,
            surface.width,
            surface.height,
        ))?;
        let Some(live) = live.filter(|e| {
            e.enabled
                && e.menu_target == expected.menu_target
                && e.name == expected.name
                && e.screen_bounds == expected.screen_bounds
        }) else {
            return Err(tracker.failure_as(
                PreCommitFailureKind::ObservationStale,
                invalid("The observed menu item changed or disappeared."),
            ));
        };
        let b = live.screen_bounds;
        let point = (b.x + (b.width / 2) as i32, b.y + (b.height / 2) as i32);
        if unsafe {
            MenuItemFromPoint(
                None,
                menu,
                POINT {
                    x: point.0,
                    y: point.1,
                },
            )
        } != native.position as i32
        {
            return Err(tracker.failure_as(
                PreCommitFailureKind::TargetOccluded,
                invalid("The popup hit test no longer resolves the observed item."),
            ));
        }
        point
    } else {
        let live = tracker.result_as(
            PreCommitFailureKind::ObservationStale,
            resolve_element_on_surface(surface, expected, surface.width, surface.height),
        )?;
        tracker.result_as(
            PreCommitFailureKind::TargetOccluded,
            live_element_point(&live, surface),
        )?
    };
    tracker.result_as(
        PreCommitFailureKind::TargetOccluded,
        ensure_point_targets_window(point, surface),
    )?;
    tracker.result_as(
        PreCommitFailureKind::ObservationStale,
        verify_surface(parent, surface, menu),
    )?;
    tracker.result(ensure_mouse_button_not_physically_pressed(
        MouseButton::Left,
    ))?;
    tracker.result(tracker.mark())?;
    tracker.result(send_mouse_click(MouseButton::Left, Some(point), tracker))
}
