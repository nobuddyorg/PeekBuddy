//! A UI Automation client for the end-to-end smoke test of the viewer (#71).
//!
//! egui publishes its widgets through AccessKit, which answers UI Automation
//! as soon as a client asks. The smoke test asks the way a screen reader
//! does: it finds the window by its handle, lists the named controls and
//! invokes one, with no knowledge of how the window is drawn. It is a COM
//! client, so it runs on a thread with its own apartment.

use std::io;

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation, IUIAutomationElement, IUIAutomationInvokePattern,
    IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern, TreeScope_Descendants,
    UIA_InvokePatternId, UIA_SelectionItemPatternId, UIA_TogglePatternId,
};
use windows::core::Interface as _;

/// `RPC_E_CHANGED_MODE`: the thread has an apartment of another kind, which
/// is usable but not ours to end.
const CHANGED_MODE: u32 = 0x8001_0106;

fn error(what: &str, source: &windows::core::Error) -> io::Error {
    io::Error::other(format!("UI Automation, {what}: {source}"))
}

/// One control of the window as a screen reader sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The `UIA_*ControlTypeId` (50000 is a button, 50002 a check box, ...).
    pub control_type: i32,
    /// The accessible name; empty for an unnamed control.
    pub name: String,
}

/// The connection to UI Automation; ends the apartment of the thread when
/// it is dropped, after every element of it.
pub struct Client {
    automation: IUIAutomation,
    ended: bool,
}

impl Drop for Client {
    fn drop(&mut self) {
        if self.ended {
            // SAFETY: balances the successful `CoInitializeEx` of `new` on
            // this thread. The caller drops every `Window` first (they
            // borrow the client), so no COM object outlives the apartment.
            unsafe { CoUninitialize() };
        }
    }
}

impl Client {
    /// Connects to UI Automation on this thread.
    ///
    /// # Errors
    ///
    /// Returns the COM error when the apartment or the automation object
    /// cannot be created.
    pub fn new() -> io::Result<Self> {
        // SAFETY: no pointers; the result is checked and `Drop` balances it.
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let ended = if result.is_ok() {
            true
        } else if result.0.cast_unsigned() == CHANGED_MODE {
            false
        } else {
            return Err(io::Error::other(format!("COM: {result:?}")));
        };
        // SAFETY: the apartment exists on this thread; no aggregation.
        let automation: windows::core::Result<IUIAutomation> =
            unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) };
        match automation {
            Ok(automation) => Ok(Self { automation, ended }),
            Err(source) => {
                if ended {
                    // SAFETY: balances the `CoInitializeEx` above; nothing
                    // was created.
                    unsafe { CoUninitialize() };
                }
                Err(error("create", &source))
            }
        }
    }

    /// The element of the top-level window with this handle.
    ///
    /// # Errors
    ///
    /// Returns the COM error when the window is not known to UI Automation.
    pub fn window(&self, hwnd: isize) -> io::Result<Window<'_>> {
        // SAFETY: `hwnd` is only looked up; a stale handle gives an error.
        let element = unsafe {
            self.automation
                .ElementFromHandle(windows::Win32::Foundation::HWND(hwnd as *mut _))
        }
        .map_err(|source| error("element from handle", &source))?;
        Ok(Window {
            client: self,
            element,
        })
    }
}

/// A window found through [`Client::window`].
pub struct Window<'a> {
    client: &'a Client,
    element: IUIAutomationElement,
}

impl Window<'_> {
    /// Every element below the window, in tree order.
    fn elements(&self) -> io::Result<Vec<IUIAutomationElement>> {
        // SAFETY: a plain COM call on the live automation object.
        let all = unsafe { self.client.automation.CreateTrueCondition() }
            .map_err(|source| error("condition", &source))?;
        // SAFETY: a plain COM call on the live element with a live condition.
        let found = unsafe { self.element.FindAll(TreeScope_Descendants, &all) }
            .map_err(|source| error("find all", &source))?;
        // SAFETY: a property read on the live collection.
        let length = unsafe { found.Length() }.map_err(|source| error("length", &source))?;
        let mut elements = Vec::new();
        for index in 0..length {
            // SAFETY: `index` is below the length just read.
            let element =
                unsafe { found.GetElement(index) }.map_err(|source| error("element", &source))?;
            elements.push(element);
        }
        Ok(elements)
    }

    /// Every control below the window, in tree order.
    ///
    /// # Errors
    ///
    /// Returns the COM error when the tree cannot be read.
    pub fn nodes(&self) -> io::Result<Vec<Node>> {
        Ok(self
            .elements()?
            .iter()
            .map(|element| Node {
                // SAFETY: a property read on a live element.
                control_type: unsafe { element.CurrentControlType() }.map_or(0, |id| id.0),
                // SAFETY: a property read on a live element.
                name: unsafe { element.CurrentName() }
                    .map_or_else(|_| String::new(), |text| text.to_string()),
            })
            .collect())
    }

    /// Invokes (clicks) the first control named `name` that can be: a button
    /// is invoked, a toggle toggled, a tab selected.
    ///
    /// # Errors
    ///
    /// Returns an error when no control has that name or it cannot be invoked.
    pub fn invoke(&self, name: &str) -> io::Result<()> {
        let mut seen = Vec::new();
        for element in self.elements()? {
            // SAFETY: a property read on a live element.
            let current = unsafe { element.CurrentName() }.map(|text| text.to_string());
            if current.as_deref() != Ok(name) {
                continue;
            }
            // SAFETY: a property read on a live element.
            seen.push(unsafe { element.CurrentControlType() }.map_or(0, |id| id.0));
            // A button invokes, a toggle button toggles, a tab is selected.
            // SAFETY: a plain COM call on a live element.
            let invoke = unsafe { element.GetCurrentPattern(UIA_InvokePatternId) }
                .and_then(|pattern| pattern.cast::<IUIAutomationInvokePattern>());
            if let Ok(pattern) = invoke {
                // SAFETY: a plain COM call on the live pattern.
                return unsafe { pattern.Invoke() }.map_err(|source| error("invoke", &source));
            }
            // SAFETY: a plain COM call on a live element.
            let toggle = unsafe { element.GetCurrentPattern(UIA_TogglePatternId) }
                .and_then(|pattern| pattern.cast::<IUIAutomationTogglePattern>());
            if let Ok(pattern) = toggle {
                // SAFETY: a plain COM call on the live pattern.
                return unsafe { pattern.Toggle() }.map_err(|source| error("toggle", &source));
            }
            // SAFETY: a plain COM call on a live element.
            let select = unsafe { element.GetCurrentPattern(UIA_SelectionItemPatternId) }
                .and_then(|pattern| pattern.cast::<IUIAutomationSelectionItemPattern>());
            if let Ok(pattern) = select {
                // SAFETY: a plain COM call on the live pattern.
                return unsafe { pattern.Select() }.map_err(|source| error("select", &source));
            }
            // A label of the same name (a tab has one next to it): try the next.
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no control named {name:?} that can be invoked or selected (types {seen:?})"),
        ))
    }
}
