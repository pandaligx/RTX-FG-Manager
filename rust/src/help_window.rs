//! A separate STA window for documentation, with no native script bridge.
use anyhow::{Result, ensure};
use std::{cell::RefCell, sync::mpsc};
use webview2_com::{
    CoTaskMemPWSTR, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, Microsoft::Web::WebView2::Win32::*,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler,
    PermissionRequestedEventHandler, WebResourceRequestedEventHandler,
};
use windows::{
    Win32::{
        Foundation::*,
        System::{Com::*, LibraryLoader::GetModuleHandleW},
        UI::WindowsAndMessaging::*,
    },
    core::{BOOL, PCWSTR, PWSTR, w},
};

thread_local! { static CONTROLLER: RefCell<Option<ICoreWebView2Controller>> = const { RefCell::new(None) }; }

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: Windows supplies a live window and message parameters to its registered procedure.
    unsafe {
        match message {
            WM_SETFOCUS => {
                CONTROLLER.with(|slot| {
                    if let Some(controller) = slot.borrow().as_ref() {
                        let _ = controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
                    }
                });
                LRESULT(0)
            }
            WM_SIZE => {
                CONTROLLER.with(|slot| {
                    if let Some(controller) = slot.borrow().as_ref() {
                        let mut bounds = RECT::default();
                        if GetClientRect(hwnd, &mut bounds).is_ok() {
                            let _ = controller.SetBounds(bounds);
                        }
                    }
                });
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: This guard exists only after successful STA initialization on this same thread.
        unsafe {
            CoUninitialize();
        }
    }
}
struct Frame(HWND);
impl Drop for Frame {
    fn drop(&mut self) {
        // SAFETY: All controllers and HWNDs belong to this STA. Close releases event callbacks.
        unsafe {
            CONTROLLER.with(|slot| {
                if let Some(controller) = slot.borrow_mut().take() {
                    let _ = controller.Close();
                }
            });
            if IsWindow(Some(self.0)).as_bool() {
                let _ = DestroyWindow(self.0);
            }
        }
    }
}
fn same_document(candidate: &str, initial: &reqwest::Url) -> bool {
    reqwest::Url::parse(candidate).is_ok_and(|url| {
        url.scheme() == "file"
            && url.host_str() == initial.host_str()
            && url.path() == initial.path()
    })
}
fn allowed_resource(candidate: &str, initial: &reqwest::Url) -> bool {
    ["index.html", "style.css", "app.js", "content.js"]
        .iter()
        .any(|name| {
            initial
                .join(name)
                .is_ok_and(|file| same_document(candidate, &file))
        })
}
fn external_link(uri: &str, initiated: BOOL) {
    if initiated.as_bool()
        && reqwest::Url::parse(uri).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str() == Some("github.com")
                && url.username().is_empty()
                && url.password().is_none()
        })
    {
        let _ = crate::win::open(uri);
    }
}

pub fn run(uri: &str) -> Result<()> {
    let initial = reqwest::Url::parse(uri)?;
    ensure!(initial.scheme() == "file", "帮助窗口只接受本地内容");
    let cache = crate::assets::cache_root()?.join("help-webview");
    crate::core::no_links(&cache)?;
    std::fs::create_dir_all(&cache)?;
    let cache = crate::win::wide(cache.as_os_str());
    // SAFETY: The help window runs on a fresh dedicated thread; all COM objects remain on it.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    }
    let _apartment = Apartment;
    // SAFETY: Static class name/procedure, valid module and UTF-16 strings, no application data pointer.
    let frame = unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance.into(),
            lpszClassName: w!("RTXFGHelpWindow"),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        // An already registered class is valid for subsequent help windows.
        RegisterClassW(&class);
        Frame(CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class.lpszClassName,
            w!("RTX FG Manager · Help"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1040,
            760,
            None,
            None,
            Some(instance.into()),
            None,
        )?)
    };
    let (tx, rx) = mpsc::channel();
    CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: NUL-terminated user data path lives through the call; callback owns its result.
            unsafe {
                CreateCoreWebView2EnvironmentWithOptions(
                    PCWSTR::null(),
                    PCWSTR(cache.as_ptr()),
                    None,
                    &handler,
                )
                .map_err(webview2_com::Error::WindowsError)
            }
        }),
        Box::new(move |status, environment| {
            status?;
            let _ = tx.send(environment);
            Ok(())
        }),
    )
    .map_err(|e| anyhow::anyhow!("WebView2: {e}"))?;
    let environment = rx
        .recv()?
        .ok_or_else(|| anyhow::anyhow!("WebView2 runtime unavailable"))?;
    let creation_environment = environment.clone();
    let hwnd = frame.0;
    let (tx, rx) = mpsc::channel();
    CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: The hidden parent window remains live until initialization completes.
            unsafe {
                creation_environment
                    .CreateCoreWebView2Controller(hwnd, &handler)
                    .map_err(webview2_com::Error::WindowsError)
            }
        }),
        Box::new(move |status, controller| {
            status?;
            let _ = tx.send(controller);
            Ok(())
        }),
    )
    .map_err(|e| anyhow::anyhow!("WebView2: {e}"))?;
    let controller = rx
        .recv()?
        .ok_or_else(|| anyhow::anyhow!("WebView2 controller unavailable"))?;
    CONTROLLER.with(|slot| *slot.borrow_mut() = Some(controller.clone()));
    // SAFETY: Live COM interfaces on their owning STA; callbacks only consume validated event data.
    unsafe {
        let webview = controller.CoreWebView2()?;
        let settings = webview.Settings()?;
        settings.SetIsWebMessageEnabled(false)?;
        settings.SetAreHostObjectsAllowed(false)?;
        settings.SetAreDevToolsEnabled(false)?;
        settings.SetAreDefaultContextMenusEnabled(false)?;
        settings.SetAreDefaultScriptDialogsEnabled(false)?;
        let mut token = 0;
        let document = initial.clone();
        webview.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let mut uri = PWSTR::null();
                    args.Uri(&mut uri)?;
                    let uri = CoTaskMemPWSTR::from(uri).to_string();
                    if !same_document(&uri, &document) {
                        args.SetCancel(true)?;
                        let mut user = BOOL(0);
                        args.IsUserInitiated(&mut user)?;
                        external_link(&uri, user);
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        webview.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    args.SetHandled(true)?;
                    let mut uri = PWSTR::null();
                    args.Uri(&mut uri)?;
                    let uri = CoTaskMemPWSTR::from(uri).to_string();
                    let mut user = BOOL(0);
                    args.IsUserInitiated(&mut user)?;
                    external_link(&uri, user);
                }
                Ok(())
            })),
            &mut token,
        )?;
        webview.add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        webview.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
        webview.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let request = args.Request()?;
                    let mut uri = PWSTR::null();
                    request.Uri(&mut uri)?;
                    let uri = CoTaskMemPWSTR::from(uri).to_string();
                    if !allowed_resource(&uri, &initial) {
                        let response = environment.CreateWebResourceResponse(
                            None,
                            403,
                            w!("Forbidden"),
                            w!("Content-Type: text/plain"),
                        )?;
                        args.SetResponse(&response)?;
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        let mut bounds = RECT::default();
        GetClientRect(hwnd, &mut bounds)?;
        controller.SetBounds(bounds)?;
        controller.SetIsVisible(true)?;
        let uri = crate::win::wide(uri);
        webview.Navigate(PCWSTR(uri.as_ptr()))?;
        let _ = ShowWindow(hwnd, SW_SHOW);
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result == 0 {
                break;
            }
            ensure!(result != -1, "帮助窗口消息处理失败");
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn help_navigation_stays_in_content_directory() {
        let url = reqwest::Url::parse("file:///C:/cache/help/abc/index.html?lang=zh").unwrap();
        assert!(same_document(
            "file:///C:/cache/help/abc/index.html#faq",
            &url
        ));
        assert!(allowed_resource("file:///C:/cache/help/abc/app.js", &url));
        for denied in [
            "https://example.com/app.js",
            "file:///C:/Windows/win.ini",
            "file:///C:/cache/help/abc/../secret.txt",
            "file:///C:/cache/help/abc/other.js",
        ] {
            assert!(!allowed_resource(denied, &url));
        }
    }
}
