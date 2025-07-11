// snake case is just bad
#![allow(non_snake_case)]

//#![feature(buf_read_has_data_left)]

use futures;


use std::time::{Duration, Instant, SystemTime};
use std::io::Read;
use vte::Parser;

use crossterm::terminal::{enable_raw_mode, disable_raw_mode};
use arboard::Clipboard;

use proc_macros::{color, load_lua_script, load_lua_scripts};

mod StringPatternMatching;
mod eventHandler;
mod TermRender;
mod CodeTabs;
mod Tokens;
mod Colors;
mod FileManager;
mod DataManager;
mod RuntimeScheduler;
mod TokenInfo;
mod languageServer;

use StringPatternMatching::*;
use Colors::*;
use eventHandler::*;

use crate::TokenInfo::*;

use CodeTabs::CodeTab;

use eventHandler::{KeyCode, KeyModifiers, KeyParser, MouseEventType};
use TermRender::{Colorize, Span, ColorType};
use FileManager::*;

use RuntimeScheduler::Runtime;
use crate::languageServer::RustAnalyzer;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum FileTabs {
    Outline,
    #[default] Files,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum TabState {
    #[default] Code,
    Files,
    Tabs,
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub enum AppState {
    Tabs,
    CommandPrompt,
    #[default] Menu,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum MenuState {
    #[default] Welcome,
    Settings
}

type LuaScripts = std::sync::Arc<parking_lot::Mutex<std::collections::HashMap <Languages, mlua::Function>>>;
pub type RustAnalyzerLsp<'a> = &'a Option <std::sync::Arc <parking_lot::RwLock <RustAnalyzer>>>;

#[derive(Debug, Default)]
pub struct App <'a> {
    exit: std::sync::Arc <parking_lot::RwLock <bool>>,
    appState: AppState,
    tabState: TabState,
    codeTabs: CodeTabs::CodeTabs,
    currentCommand: String,
    fileBrowser: FileBrowser,
    area: TermRender::Rect,
    lastScrolled: u128,

    debugInfo: String,
    suggested: String,

    preferredCommandKeybind: KeyModifiers,
    colorMode: ColorMode <'a>,

    menuState: MenuState,

    currentDir: String,
    dirFiles: Vec<String>,

    currentMenuSettingBox: usize,

    lastTab: usize,

    luaSyntaxHighlightScripts: LuaScripts,

    // no longer needed. All operations that happened here are now running on threads inside the runtime manager
    //mainThreadHandles: Vec <std::thread::JoinHandle<()>>,

    dtScalar: f64,

    allFiles: Vec <FileInfo>,

    lastTabName: String,
}

impl <'a> App <'a> {
    /// runs the application's main loop until the user quits
    pub async fn Run (&mut self,
                      app: &mut TermRender::App,
                      runtime: std::sync::Arc <parking_lot::RwLock <Runtime>>
    ) -> Result<(), std::io::Error> {
        enable_raw_mode()?; // Enable raw mode for direct input handling

        // making sure the lsp can immediately be connected without having to wait
        let mut lastPolled = Instant::now() - Duration::new(30,0);
        let mut rustAnalyzerInstance = None;

        let terminalSize = app.GetTerminalSize()?;
        self.area = TermRender::Rect {
            width: terminalSize.0,
            height: terminalSize.1
        };

        load_lua_script!(
            self.luaSyntaxHighlightScripts,
            Languages::Null,
            "assets/nullSyntaxHighlighting.lua",
        );

        load_lua_scripts!(
            self.luaSyntaxHighlightScripts,
            "data/syntaxHighlighting.json",
        );

        let mut stdout = std::io::stdout();
        crossterm::execute!(stdout, crossterm::terminal::Clear(crossterm::terminal::ClearType::All))?;
        
        let mut clipboard = Clipboard::new().expect("Failed to initialize clipboard");

        let parser = std::sync::Arc::new(parking_lot::RwLock::new(Parser::new()));
        let keyParser = std::sync::Arc::new(parking_lot::RwLock::new(KeyParser::new()));
        let buffer = std::sync::Arc::new(parking_lot::RwLock::new([0; 128]));  // [0; 10]; not sure how much the larger buffer is actually helping
        let stdin = std::sync::Arc::new(parking_lot::RwLock::new(std::io::stdin()));

        self.HandleMainLoop(
            runtime.clone(),
            buffer.clone(),
            keyParser.clone(),
            stdin.clone(),
            parser.clone(),
        ).await?;

        let exit = self.exit.clone();

        loop {
            let start = SystemTime::now();

            let exitRead = *exit.read();
            if exitRead {  break;  }

            self.HandleRustAnalyzer(&mut rustAnalyzerInstance, &runtime, &mut lastPolled);

            buffer.write().fill(0);

            self.codeTabs.CheckScopeThreads();  // no sure how this went missing....

            let _updates = self.RenderFrame(app);  // ignoring the redraw count (mostly used/needed for debugging)

            let termSize = app.GetTerminalSize()?;
            self.area = TermRender::Rect {
                width: termSize.0,
                height: termSize.1,
            };  // ig this is a thing

            // the .read is ugly, but whatever. It's probably fine if polling stops while
            // processing the events
            self.HandleKeyEvents(&keyParser.read(), &mut clipboard, &rustAnalyzerInstance).await;
            self.HandleMouseEvents(&keyParser.read()).await;  // not sure if this will be delayed, but I think it should work? idk
            self.HandleMixedEvents(&keyParser.read(), &rustAnalyzerInstance).await;
            keyParser.write().ClearEvents();

            let end = SystemTime::now();
            let elapsedTime = end.duration_since(start).unwrap().as_micros() as f64 * 0.000001;  // in seconds
            const FPS_AIM: f64 = 1f64 / 60f64;  // the target fps (forces it to stall to this to ensure low CPU usage)
            let difference = (FPS_AIM - elapsedTime).max(0f64) * 0.9;
            self.dtScalar = 1.0 / (FPS_AIM / (elapsedTime + difference));  // for dt scaling

            // this usage is an ok exception to the standard/usual restrictions.
            // The custom async runtime manager assigns a unique thread to each async task,
            // so regardless of the sleep function, a blocking or non-blocking sleep call
            // won't impact the executor's ability to poll other futures.
            std::thread::sleep(Duration::from_secs_f64(difference));
            //if self.codeTabs.tabs.is_empty() {  continue;  }
            //self.debugInfo = format!("{}; {} | {} | {}", _updates, self.codeTabs.tabs[self.lastTab].resetCache.len(), self.codeTabs.tabs[self.lastTab].scrollCache.len(), self.codeTabs.tabs[self.lastTab].shiftCache);
        }

        disable_raw_mode()?;

        Ok(())
    }

    fn HandleRustAnalyzer (&mut self,
                           rustAnalyzer: &mut Option <std::sync::Arc <parking_lot::RwLock <RustAnalyzer>>>,
                           runtime: &std::sync::Arc <parking_lot::RwLock <Runtime>>,
                           lastPolled: &mut Instant
    ) {
        if self.appState == AppState::Menu {
            self.debugInfo = String::from("Waiting...");
            // .*lastPolled = std::time::Instant::now();
            return;
        }
        // trying to connect with the lsp (every 10 seconds)
        if rustAnalyzer.is_none() {
            let currentTime = Instant::now();  // once connected this will no longer be called
            if currentTime.duration_since(*lastPolled).as_secs_f64() > 15.0 {
                self.debugInfo = String::from("Trying to connect...");
                let pathway = self.fileBrowser.fileTree.pathName.clone();
                *rustAnalyzer = self.CreateRustAnalyzerInterface(&runtime, pathway);
                *lastPolled = Instant::now();
            } else {
                self.debugInfo = String::from("Waiting to connect...");
            }
        } else {
            self.debugInfo = String::from("Connected");

            // going through the responses and handling them
            // handling timeouts incase the polling process uses the instance for too long; this should prevent frame stutters
            let analyzer = rustAnalyzer.as_mut().unwrap().try_write_for(Duration::from_millis(25));
            if analyzer.is_none() {  return;  }  // incase a timeout happens to prevent stalling
            let mut analyzer = analyzer.unwrap();
            while let Some(_event) = analyzer.PopResponse() {
                // todo!   handle the event
            }
            // updating the filepath to ensure events are correctly handled
            if self.codeTabs.tabs.is_empty() {  return;  }
            *analyzer.filePath.1.write() = self.codeTabs.tabs[self.lastTab].path.clone();
        }
    }

    async fn HandleMixedEvents<'b> (&mut self, keyEvents: &KeyParser, rustAnalyzer: RustAnalyzerLsp<'b>) {
        self.PressedLoadFile(keyEvents, rustAnalyzer).await;
    }

    fn GetEventHandlerHandle (&mut self,
                              runtime: std::sync::Arc <parking_lot::RwLock <Runtime>>,
                              buffer: std::sync::Arc <parking_lot::RwLock <[u8; 128]>>,
                              keyParser: std::sync::Arc <parking_lot::RwLock <KeyParser>>,
                              stdin: std::sync::Arc <parking_lot::RwLock <std::io::Stdin>>,
                              parser: std::sync::Arc <parking_lot::RwLock <Parser>>,
                              exit: std::sync::Arc <parking_lot::RwLock<bool >>,
    ) {
        let buffer = buffer.clone();
        let keyParser = keyParser.clone();
        let parser = parser.clone();
        let stdin = stdin.clone();

        runtime.write().AddTask(Box::pin(async move {
            loop {
                if *exit.read() {  break;  }
                let mut localBuffer = [0; 128];
                let result = stdin.write().read(&mut localBuffer);
                if let Ok(n) = result {
                    *buffer.write() = localBuffer;
                    keyParser.write().bytes = n;

                    if n == 1 && buffer.read()[0] == 0x1B {
                        keyParser.write().keyEvents.insert(KeyCode::Escape, true);
                    } else {
                        parser.write().advance(&mut *keyParser.write(), &buffer.read()[..n]);
                    }
                }
                futures::pending!();  // yielding to the executor
            }
        }));
    }

    fn CreateRustAnalyzerInterface (&mut self,
                                    runtime: &std::sync::Arc <parking_lot::RwLock <Runtime>>,
                                    filePath: String,
    ) -> Option <std::sync::Arc <parking_lot::RwLock <RustAnalyzer>>> {
        // only creating an instance if rust analyzer is found and able to be communicated with


        // this is what's causing the stalling upon opening a file
        let rustAnalyzer = RustAnalyzer::new(filePath);  // this will block the main thread until completion....

        if let Some(rustAnalyzer) = rustAnalyzer {
            let rustAnalyzer = std::sync::Arc::new(parking_lot::RwLock::new(rustAnalyzer));
            let rustAnalyzerClone = rustAnalyzer.clone();
            // creating a unique runtime for the lsp (communication will be done through the Arc <rustAnalyzer>)
            let HandleLspErrors = Self::HandleLspErrors;
            runtime.write().AddTask(Box::pin(async move {
                // the runtime to manage/handle all lsp communication
                let mut errorCount: Vec <Instant> = vec![];
                let mut iterationCount = 0;
                loop {
                    if iterationCount > languageServer::SYNCHRONIZE_COUNT {
                        rustAnalyzerClone.write().NewEvent(languageServer::RustEvents::Synchronize);
                        iterationCount = 0;  // resetting the count
                    }
                    let status = rustAnalyzerClone.write().Poll().await;
                    match status {
                        languageServer::ExitStatus::ResponseTimeOut (event) => {
                            // for now nothing is done.....
                            // todo! eventually maybe add tracking for the occurrence rate
                            //-- if the rate is too high than maybe figure out a way to try and
                            // reestablish a connection or report the disconnection; a true disconnection
                            // seems to throw an error which would be ExitStatus::Error? Or maybe check
                            // the type of event or something to try and diagnose or avoid the problem (
                            // or provide feedback to the user on where the issue is occurring)
                            // errors seem to be separate though? Do I need to track this?

                            // re-appending the event to ensure it will eventually be handled
                            // the event is pushed to the back of the queue incase it continues to fail
                            // so that hopefully it won't completely clog up the rest of the requests
                            // the old event will finish being parsed async-safe-blocking further polling till then
                            rustAnalyzerClone.write().NewEventBack(event);
                        },
                        languageServer::ExitStatus::Error => {
                            // handling the error; this may be a blocking task, but everything
                            // is handled safely to do such. Safe connection drops are still ensured
                            // regardless of its blocking status
                            HandleLspErrors(&mut errorCount, &rustAnalyzerClone).await;
                        },
                        languageServer::ExitStatus::ReadingBlockedTimeOut => {
                            // do something or not, idk
                        },
                        _ => {}  // ignoring other responses (such as Valid)
                    }
                    // the lsp will be updated every 1/10 of a second to avoid excessive cpu usage
                    // blocking the async task is safe using this runtime; it won't block other futures from executing
                    std::thread::sleep(Duration::from_millis(100));
                    futures::pending!();  // should yield the task at some point...
                    iterationCount += 1;
                }

            }));
            return Some(rustAnalyzer);
        } None
    }

    // this actually seems to properly disconnect and reconnect, wow
    async fn HandleLspErrors(errorCount: &mut Vec <Instant>,
                             rustAnalyzer: &std::sync::Arc <parking_lot::RwLock <RustAnalyzer>>,
    ) {
        let currentTime = Instant::now();
        errorCount.push(currentTime.clone());
        for _ in 0..errorCount.len() {
            if currentTime.duration_since(errorCount[0]).as_secs_f64() > 45.0 {
                errorCount.remove(0);
            }
        }

        // if there are 5 errors over 45 seconds, then restart the lsp connection
        // hopefully this will ensure it's actually a prolonged outage and not short term drop
        if errorCount.len() >= 5 {
            // replacing the instance and dropping the old one
            // this should re-initialize a new instance of the rust-analyzer binary
            let filePath = rustAnalyzer.read().filePath.0.clone();
            // because this is dropped, this thread doesn't require a clean shutdown during this
            // therefor, blocking tasks are safe
            rustAnalyzer.write().DropConnection();  // dropping the connection to ensure safety
            let mut newInstance = None;
            while newInstance.is_none() {
                // waiting 5.0 seconds between each attempt to prevent excessive cpu usage
                // the long wait is safe because of the runtime manager and because the connection
                // was already safely dropped using .DropConnection
                std::thread::sleep(Duration::from_millis(5000));
                // creating the instance after waiting so if a valid instance is created as
                // the application is closed or crashes, it should still safely drop the connection
                newInstance = RustAnalyzer::new(filePath.clone());
                futures::pending!();  // awaiting anyway (even though blocking tasks are safe in this context)
            }
            // replacing the instance; in .drop .DropConnection shouldn't be called because it already was
            let mut lock = rustAnalyzer.write();
            *lock = newInstance.unwrap();
        }
    }

    async fn HandleMainLoop (&mut self,
                             runtime: std::sync::Arc <parking_lot::RwLock <Runtime>>,
                             buffer: std::sync::Arc <parking_lot::RwLock <[u8; 128]>>,
                             keyParser: std::sync::Arc <parking_lot::RwLock <KeyParser>>,
                             stdin: std::sync::Arc <parking_lot::RwLock <std::io::Stdin>>,
                             parser: std::sync::Arc <parking_lot::RwLock <Parser>>,
    ) -> Result <(), std::io::Error> {
        let exit = self.exit.clone();
        self.GetEventHandlerHandle(
            runtime,
            buffer,
            keyParser,
            stdin,
            parser,
            exit,
        );
        //self.mainThreadHandles.push(eventThreadHandler);

        Ok(())
    }

    fn HandleScrollEvent (&mut self, event: &MouseEvent, events: &KeyParser) {
        if event.position.0 > 29 && event.position.1 < 10 + self.area.height && event.position.1 > 2 {
            let mut tabIndex = 0;
            tabIndex = self.codeTabs.GetTabNumber(
                &self.area, 29,
                event.position.0 as usize,
                &mut tabIndex
            );

            self.codeTabs.tabs[tabIndex].UpdateScroll(events.scrollAccumulate * self.dtScalar);
            let currentTime = SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .expect("Time went backwards...")
                .as_millis();
            self.lastScrolled = currentTime;
        }
    }

    fn PressedCode (&mut self, events: &KeyParser, event: &MouseEvent, padding: usize) {
        self.lastTab = self.codeTabs.GetTabNumber(
            &self.area, padding,
            event.position.0 as usize,
            &mut self.lastTab
        );
        let currentTime = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("Time went backwards...")
            .as_millis();
        self.codeTabs.tabs[self.lastTab].pauseScroll = currentTime;
        // updating the highlighting position
        if events.ContainsMouseModifier(KeyModifiers::Shift)
        {
            if !self.codeTabs.tabs[self.lastTab].highlighting {
                self.codeTabs.tabs[self.lastTab].cursorEnd =
                    self.codeTabs.tabs[self.lastTab].cursor;
                self.codeTabs.tabs[self.lastTab].highlighting = true;
            }
        } else {
            self.codeTabs.tabs[self.lastTab].highlighting = false;
        }

        // adjusting the position for panes
        let position = (
            self.codeTabs.GetRelativeTabPosition(event.position.0, &self.area, padding as u16 + 4),
            event.position.1
        );

        let tab = &mut self.codeTabs.tabs[self.lastTab];
        let lineSize = tab.lines.len().to_string().len();  // account for the length of the total lines
        let linePos = (std::cmp::max(tab.scrolled as isize + tab.mouseScrolled, 0) as usize +
                           position.1.saturating_sub(3) as usize,
                       position.0.saturating_sub(lineSize as u16) as usize);
        tab.cursor = (
            std::cmp::min(
                linePos.0,
                tab.lines.len() - 1
            ),
            linePos.1.saturating_sub( {
                if linePos.0 == tab.cursor.0 && linePos.1 > tab.cursor.1 {
                    1
                } else {  0  }
            } )
        );
        tab.cursor.1 = std::cmp::min(
            tab.cursor.1,
            tab.lines[tab.cursor.0].len()
        );
        tab.scrolled = std::cmp::max(tab.mouseScrolledFlt as isize + tab.scrolled as isize, 0) as usize;
        tab.mouseScrolled = 0;
        tab.mouseScrolledFlt = 0.0;
        self.appState = AppState::Tabs;
        self.tabState = TabState::Code;
    }

    fn PressedScopeJump (&mut self, _events: &KeyParser, event: &MouseEvent) {
        // getting the line clicked on and jumping to it if it's in range
        // account for the line scrolling/shifting... (not as bad as I thought it would be)
        let scrollTo = self.fileBrowser.outlineCursor.saturating_sub(((self.area.height - 8) / 2) as usize);
        let line = std::cmp::min(
            event.position.1.saturating_sub(3) as usize + scrollTo,
            self.codeTabs.tabs[self.lastTab].linearScopes.read().len() - 1
        );
        self.fileBrowser.outlineCursor = line;
        let scopes = &mut self.codeTabs.tabs[self.lastTab].linearScopes.read()[
            line].clone();
        scopes.reverse();
        let newCursor = self.codeTabs.tabs[self.lastTab]
            .scopes
            .read()
            .GetNode(
                scopes
            ).start;
        self.codeTabs.tabs[self.lastTab].cursor.0 = newCursor;
        self.codeTabs.tabs[self.lastTab].mouseScrolled = 0;
        self.codeTabs.tabs[self.lastTab].mouseScrolledFlt = 0.0;
    }

    fn PressedNewCodeTab (&mut self, events: &KeyParser, event: &MouseEvent) {
        // tallying the size till the correct tab is found
        let mut sizeCounted = 29usize;
        for (index, tab) in self.codeTabs.tabFileNames.iter().enumerate() {
            sizeCounted += 6 + (index + 1).to_string().len() + tab.len();
            if !self.codeTabs.tabs[index].saved {  sizeCounted += 1;  }
            if sizeCounted >= event.position.0 as usize {
                if events.ContainsMouseModifier(KeyModifiers::Shift) {
                    self.codeTabs.panes.push(index);
                } else {
                    self.codeTabs.currentTab = index;
                    self.lastTab = index;
                }
                break;
            }
        }
    }

    async fn HandlePress (&mut self, events: &KeyParser, event: &MouseEvent, padding: u16) {
        if  event.position.0 > padding &&
            event.position.1 < self.area.height - 8 &&
            event.position.1 > 2 &&
            !self.codeTabs.tabs.is_empty()
        {
            self.PressedCode(events, event, padding as usize);
        } else if
        event.position.0 <= 29 &&
            event.position.1 < self.area.height - 10 &&
            self.fileBrowser.fileTab == FileTabs::Outline &&
            !self.codeTabs.tabs.is_empty() &&
            self.fileBrowser.fileOptions.selectedOptionsTab == OptionTabs::Null
        {
            self.PressedScopeJump(events, event);
        } else if
        event.position.0 > 29 &&
            event.position.1 <= 2 &&
            !self.codeTabs.tabs.is_empty()
        {
            self.PressedNewCodeTab(events, event);
        }
    }

    fn HighlightLeftClick (&mut self, _events: &KeyParser, event: &MouseEvent, padding: usize) {
        // updating the highlighting position
        self.lastTab = self.codeTabs.GetTabNumber(
            &self.area, padding,
            event.position.0 as usize,
            &mut self.lastTab
        );
        // adjusting the position
        let position = (
            self.codeTabs.GetRelativeTabPosition(event.position.0, &self.area, padding as u16 + 4),
            event.position.1
        );

        let cursorEnding = self.codeTabs.tabs[self.lastTab].cursor;

        let tab = &mut self.codeTabs.tabs[self.lastTab];
        let lineSize = tab.lines.len().to_string().len();  // account for the length of the total lines
        let linePos = (std::cmp::max(tab.scrolled as isize + tab.mouseScrolled, 0) as usize +
                           position.1.saturating_sub(3) as usize,
                       position.0.saturating_sub(lineSize as u16) as usize);
        tab.cursor = (
            std::cmp::min(
                linePos.0,
                tab.lines.len() - 1
            ),
            linePos.1.saturating_sub( {
                if linePos.0 == tab.cursor.0 && linePos.1 > tab.cursor.1 {
                    1
                } else {  0  }
            } )
        );
        tab.cursor.1 = std::cmp::min(
            tab.cursor.1,
            tab.lines[tab.cursor.0].len()
        );
        tab.mouseScrolled = 0;
        tab.mouseScrolledFlt = 0.0;
        self.appState = AppState::Tabs;
        self.tabState = TabState::Code;

        if cursorEnding != tab.cursor && !tab.highlighting
        {
            if !tab.highlighting {
                tab.cursorEnd = cursorEnding;
                tab.highlighting = true;
            }
        } else if !tab.highlighting {
            self.codeTabs.tabs[self.lastTab].highlighting = false;
        }
    }

    async fn HandleLeftClick (&mut self, events: &KeyParser, event: &MouseEvent) {
        let padding =
            if self.appState == AppState::CommandPrompt {  29  }
            else {  0  };
        // checking for code selection
        if matches!(event.state, MouseState::Release | MouseState::Hold) {
            if event.position.0 > padding && event.position.1 < self.area.height - 8 &&
                event.position.1 > 2 &&
                !self.codeTabs.tabs.is_empty()
            {
                self.HighlightLeftClick(events, event, padding as usize);
            }
        } else if event.state == MouseState::Press {
            self.HandlePress(events, event, padding).await;
        }
    }

    fn HandleMenuMouseEvents (&mut self, _events: &KeyParser, _event: &MouseEvent) {
        // todo!
    }

    async fn HandleMouseEvents (&mut self, events: &KeyParser) {
        if let Some(event) = &events.mouseEvent {
            if self.appState == AppState::Menu {
                self.HandleMenuMouseEvents(events, event);
                return;
            }

            match event.eventType {
                MouseEventType::Down if !self.codeTabs.tabs.is_empty() => {
                    let currentTime = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .expect("Time went backwards...")
                        .as_millis();
                    if currentTime.saturating_sub(self.lastScrolled) < 8
                        {  return;  }
                    self.HandleScrollEvent(event, events);
                },
                MouseEventType::Up if !self.codeTabs.tabs.is_empty() => {
                    let currentTime = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .expect("Time went backwards...")
                        .as_millis();
                    if currentTime.saturating_sub(self.lastScrolled) < 8
                        {  return;  }
                    self.HandleScrollEvent(event, events);
                },
                MouseEventType::Left => {
                    self.HandleLeftClick(events, event).await;
                },
                MouseEventType::Middle => {},
                MouseEventType::Right => {},
                _ => {},
            }
        }
    }

    fn HandleTabViewTabPress (&mut self, keyEvents: &KeyParser) {
        if keyEvents.ContainsKeyCode(KeyCode::Tab) {
            if keyEvents.ContainsModifier(&KeyModifiers::Shift) &&
                self.tabState == TabState::Files {

                self.fileBrowser.fileTab = match self.fileBrowser.fileTab {
                    FileTabs::Files => FileTabs::Outline,
                    FileTabs::Outline => FileTabs::Files,
                }
            } else {
                self.tabState = match self.tabState {
                    TabState::Code => TabState::Files,
                    TabState::Files => TabState::Tabs,
                    TabState::Tabs => TabState::Code,
                }
            }
        }
    }

    fn HandleCommandPromptKeyEvents (&mut self, keyEvents: &KeyParser) {
        if keyEvents.ContainsModifier(&KeyModifiers::Option) {  return;  }
        for chr in &keyEvents.charEvents {
            self.currentCommand.push(*chr);
        }

        self.HandleTabViewTabPress(keyEvents);
        self.HandleCodeTabviewKeyEvents(keyEvents);

        match self.tabState {
            TabState::Code => {
                // nothing so far ig
            },
            TabState::Files => {
                self.HandleFilebrowserKeyEvents(keyEvents);
            },
            TabState::Tabs => {
                self.HandleTabsKeyEvents(keyEvents);
            },
        }

        if !self.currentCommand.is_empty() {
            self.HandleCommands(keyEvents);
        }
    }

    fn JumpLineUp (&mut self) {
        // jumping up
        if let Some(numberString) = self.currentCommand.get(1..) {
            let number = numberString.parse:: <usize>();
            if number.is_ok() {
                let cursor = self.codeTabs.tabs[self.lastTab].cursor.0;
                self.codeTabs.tabs[self.lastTab].JumpCursor(
                    cursor.saturating_sub(number.unwrap_or(0)), 1
                );
            }
        }
    }

    fn JumpLineDown (&mut self) {
        // jumping down
        if let Some(numberString) = self.currentCommand.get(1..) {
            let number = numberString.parse:: <usize>();
            if number.is_ok() {
                let cursor = self.codeTabs.tabs[self.lastTab].cursor.0;
                self.codeTabs.tabs[self.lastTab].JumpCursor(
                    cursor.saturating_add(number.unwrap_or(0)), 1
                );
            }
        }
    }

    fn HandleCommands (&mut self, keyEvents: &KeyParser) {
        if keyEvents.ContainsKeyCode(KeyCode::Return) {
            if self.currentCommand == "q" {
                self.Exit();
            }

            // jumping to, command
            if self.currentCommand.starts_with('[') {
                self.JumpLineUp();
            } else if self.currentCommand.starts_with(']') {
                self.JumpLineDown();
            } else if self.currentCommand == *"gd" {
                // todo!
            } else if self.currentCommand == *"-light" {
                TermRender::ColorMode::ToLight();
            } else if self.currentCommand == *"-dark" {
                TermRender::ColorMode::ToDark();
            }

            self.currentCommand.clear();
        } else if keyEvents.ContainsKeyCode(KeyCode::Delete) {
            self.currentCommand.pop();
        }
    }

    fn HandleCodeTabviewKeyEvents (&mut self, keyEvents: &KeyParser) {
        if keyEvents.ContainsKeyCode(KeyCode::Return) && self.currentCommand.is_empty() && !self.codeTabs.tabs.is_empty() {
            self.appState = AppState::Tabs;
        }
    }

    fn HandleFilebrowserKeyEvents (&mut self, keyEvents: &KeyParser) {
        if self.fileBrowser.fileTab == FileTabs::Outline {
            if keyEvents.ContainsKeyCode(KeyCode::Return) && self.currentCommand.is_empty() && !self.codeTabs.tabs.is_empty() {
                let mut nodePath = self.codeTabs.tabs[self.lastTab].linearScopes.read()[
                    self.fileBrowser.outlineCursor].clone();
                nodePath.reverse();
                let scopesRead = self.codeTabs.tabs[self.lastTab].scopes.read();
                let start: usize;
                {
                    let node = scopesRead.GetNode(&mut nodePath);
                    start = node.start;
                }
                drop(scopesRead);  // dropped the read
                self.codeTabs.tabs[self.lastTab].JumpCursor(start, 1);
            } else if keyEvents.ContainsKeyCode(KeyCode::Up) {
                self.fileBrowser.MoveCursorUp();
            } else if keyEvents.ContainsKeyCode(KeyCode::Down) {
                self.fileBrowser.MoveCursorDown(
                    &self.codeTabs.tabs[self.lastTab].linearScopes.read(),
                    &self.codeTabs.tabs[self.lastTab].scopes.read());
            }
        }
    }

    fn HandleTabsKeyEvents (&mut self, keyEvents: &KeyParser) {
        if keyEvents.ContainsKeyCode(KeyCode::Left) {
            if keyEvents.ContainsModifier(&KeyModifiers::Option) {
                self.codeTabs.MoveTabLeft()
            } else {
                self.codeTabs.TabLeft();
            }
        } else if keyEvents.ContainsKeyCode(KeyCode::Right) {
            if keyEvents.ContainsModifier(&KeyModifiers::Option) {
                self.codeTabs.MoveTabRight()
            } else {
                self.codeTabs.TabRight();
            }
        } else if keyEvents.ContainsKeyCode(KeyCode::Return) && !self.codeTabs.tabs.is_empty() {
            self.appState = AppState::Tabs;
            self.tabState = TabState::Code;
        } else if keyEvents.ContainsKeyCode(KeyCode::Delete) {
            self.codeTabs.tabs.remove(self.codeTabs.currentTab);
            self.codeTabs.tabFileNames.remove(self.codeTabs.currentTab);
            self.codeTabs.currentTab = self.codeTabs.currentTab.saturating_sub(1);
            if self.lastTab >= self.codeTabs.tabs.len() {
                self.lastTab = self.codeTabs.tabs.len().saturating_sub(1);
            }
        }
    }

    async fn TypeCode<'b> (&mut self,
                           keyEvents: &KeyParser,
                           _clipBoard: &mut Clipboard,
                           rustAnalyzer: RustAnalyzerLsp<'b>
    ) {
        // making sure command + s or other commands aren't being pressed
        if !(keyEvents.ContainsModifier(&KeyModifiers::Command) ||
            keyEvents.ContainsModifier(&KeyModifiers::Control) ||
            keyEvents.ContainsModifier(&KeyModifiers::Option)
        ) {
            for chr in &keyEvents.charEvents {
                if *chr == '(' {
                    self.codeTabs.tabs[self.lastTab]
                        .InsertChars("()".to_string(), &self.luaSyntaxHighlightScripts, rustAnalyzer).await;
                    self.codeTabs.tabs[self.lastTab].cursor.1 -= 1;
                } else if *chr == '{' {
                    self.codeTabs.tabs[self.lastTab]
                        .InsertChars("{}".to_string(), &self.luaSyntaxHighlightScripts, rustAnalyzer).await;
                    self.codeTabs.tabs[self.lastTab].cursor.1 -= 1;
                } else if *chr == '[' {
                    self.codeTabs.tabs[self.lastTab]
                        .InsertChars("[]".to_string(), &self.luaSyntaxHighlightScripts, rustAnalyzer).await;
                    self.codeTabs.tabs[self.lastTab].cursor.1 -= 1;
                } else if *chr == '\"' {
                    self.codeTabs.tabs[self.lastTab]
                        .InsertChars("\"\"".to_string(), &self.luaSyntaxHighlightScripts, rustAnalyzer).await;
                    self.codeTabs.tabs[self.lastTab].cursor.1 -= 1;
                } else {
                    self.codeTabs.tabs[self.lastTab]
                        .InsertChars(chr.to_string(), &self.luaSyntaxHighlightScripts, rustAnalyzer).await;
                }
            }
        }
    }

    async fn DeleteCode<'b> (&mut self,
                             keyEvents: &KeyParser,
                             _clipBoard: &mut Clipboard,
                             rustAnalyzer: RustAnalyzerLsp<'b>
    ) {
        let mut numDel = 1;
        let mut offset = 0;

        if keyEvents.keyModifiers.contains(&KeyModifiers::Option) {
            if keyEvents.ContainsModifier(&KeyModifiers::Shift) {
                numDel = self.codeTabs.tabs[self.lastTab].FindTokenPosRight();
                offset = numDel;
            } else {
                numDel = self.codeTabs.tabs[self.lastTab].FindTokenPosLeft();
            }
        } else if keyEvents.ContainsModifier(&self.preferredCommandKeybind) {
            if keyEvents.ContainsModifier(&KeyModifiers::Shift) {
                numDel = self.codeTabs.tabs[self.lastTab].lines[
                    self.codeTabs.tabs[self.lastTab].cursor.0
                    ].len() - self.codeTabs.tabs[self.lastTab].cursor.1;
                offset = numDel;
            } else {
                numDel = self.codeTabs.tabs[self.lastTab].cursor.1;
            }
        } else if keyEvents.ContainsModifier(&KeyModifiers::Shift) {
            offset = numDel;
        }

        self.codeTabs.tabs[
            self.lastTab
        ].DelChars(numDel, offset, &self.luaSyntaxHighlightScripts, rustAnalyzer).await;
    }

    fn CloseCodePane (&mut self) {
        if self.codeTabs.panes.contains(&self.lastTab) {
            self.codeTabs.panes.remove(
                self.codeTabs.panes
                    .iter()
                    .position(|e| e == &self.lastTab)
                    .unwrap()  // the if block ensures the element exists
            );
        }
    }

    fn MoveCodeCursorLeft (&mut self, keyEvents: &KeyParser, _clipBoard: &mut Clipboard) {
        let highlight= self.HandleHighlightOnCursorMove(keyEvents);

        if keyEvents.ContainsModifier(&KeyModifiers::Option) {
            self.codeTabs.tabs[self.lastTab].MoveCursorLeftToken();
        } else if keyEvents.ContainsModifier(&self.preferredCommandKeybind) {
            self.codeTabs.tabs[self.lastTab].mouseScrolledFlt = 0.0;
            self.codeTabs.tabs[self.lastTab].mouseScrolled = 0;
            // checking if it's the true first value or not
            let mut indentIndex = 0usize;
            let cursorLine = self.codeTabs.tabs[self.lastTab].cursor.0;
            for chr in self.codeTabs.tabs[self.lastTab].lines[cursorLine].chars() {
                if chr != ' ' {
                    break;
                } indentIndex += 1;
            }

            if self.codeTabs.tabs[self.lastTab].cursor.1 <= indentIndex {
                self.codeTabs.tabs[self.lastTab].cursor.1 = 0;
            } else {
                self.codeTabs.tabs[self.lastTab].cursor.1 = indentIndex;
            }
        } else {
            self.codeTabs.tabs[self.lastTab].MoveCursorLeft(1, highlight);
        }
    }

    fn HandleHighlightOnCursorMove (&mut self, keyEvents: &KeyParser) -> bool {
        if keyEvents.ContainsModifier(&KeyModifiers::Shift)
        {
            if !self.codeTabs.tabs[self.lastTab].highlighting {
                self.codeTabs.tabs[self.lastTab].cursorEnd =
                    self.codeTabs.tabs[self.lastTab].cursor;
                self.codeTabs.tabs[self.lastTab].highlighting = true;
            } true
        } else {
            false
        }
    }

    fn MoveCodeCursorRight (&mut self, keyEvents: &KeyParser, _clipBoard: &mut Clipboard) {
        let highlight = self.HandleHighlightOnCursorMove(keyEvents);

        if keyEvents.ContainsModifier(&KeyModifiers::Option) {
            self.codeTabs.tabs[self.lastTab].MoveCursorRightToken();
        } else if keyEvents.ContainsModifier(&self.preferredCommandKeybind) {
            let tab = &mut self.codeTabs.tabs[self.lastTab];
            tab.scrolled = std::cmp::max(tab.mouseScrolledFlt as isize + tab.scrolled as isize, 0) as usize;
            tab.mouseScrolledFlt = 0.0;
            tab.mouseScrolled = 0;

            let cursorLine = self.codeTabs.tabs[self.lastTab].cursor.0;
            self.codeTabs.tabs[self.lastTab].cursor.1 =
                self.codeTabs.tabs[self.lastTab].lines[cursorLine].len();
        } else {
            self.codeTabs.tabs[self.lastTab].MoveCursorRight(1, highlight);
        }
    }

    fn MoveCodeCursorUp (&mut self, keyEvents: &KeyParser, _clipBoard: &mut Clipboard) {
        let highlight = self.HandleHighlightOnCursorMove(keyEvents);

        if keyEvents.ContainsModifier(&KeyModifiers::Option) {
            let tab = &mut self.codeTabs.tabs[self.lastTab];
            let mut jumps = tab.scopeJumps.read()[tab.cursor.0].clone();
            jumps.reverse();
            let start = tab.scopes.read().GetNode(&mut jumps).start;
            tab.JumpCursor(
                start, 1
            );
        } else if keyEvents.ContainsModifier(&self.preferredCommandKeybind) {
            let tab = &mut self.codeTabs.tabs[self.lastTab];
            tab.scrolled = std::cmp::max(tab.mouseScrolledFlt as isize + tab.scrolled as isize, 0) as usize;
            tab.mouseScrolledFlt = 0.0;
            tab.mouseScrolled = 0;
            tab.cursor.0 = 0;
        } else {
            self.codeTabs.tabs[self.lastTab].CursorUp(highlight);
        }
    }

    fn MoveCodeCursorDown (&mut self, keyEvents: &KeyParser, _clipBoard: &mut Clipboard) {
        let highlight = self.HandleHighlightOnCursorMove(keyEvents);

        if keyEvents.ContainsModifier(&KeyModifiers::Option) {
            let tab = &mut self.codeTabs.tabs[self.lastTab];
            let mut jumps = tab.scopeJumps.read()[tab.cursor.0].clone();
            jumps.reverse();
            let end = tab.scopes.read().GetNode(&mut jumps).end;
            tab.JumpCursor(end, 1);
        } else if keyEvents.ContainsModifier(&self.preferredCommandKeybind) {
            let tab = &mut self.codeTabs.tabs[self.lastTab];
            tab.scrolled = std::cmp::max(tab.mouseScrolledFlt as isize + tab.scrolled as isize, 0) as usize;
            tab.mouseScrolledFlt = 0.0;
            tab.mouseScrolled = 0;
            tab.cursor.0 =
                tab.lines.len() - 1;
        } else {
            self.codeTabs.tabs[self.lastTab].CursorDown(highlight);
        }
    }

    async fn HandleCodeTabPress<'b> (&mut self,
                                     keyEvents: &KeyParser,
                                     _clipBoard: &mut Clipboard,
                                     rustAnalyzer: RustAnalyzerLsp<'b>
    ) {
        if keyEvents.ContainsModifier(&KeyModifiers::Shift) {
            self.codeTabs.tabs[self.lastTab].UnIndent(&self.luaSyntaxHighlightScripts, rustAnalyzer).await;
        } else {
            if self.suggested.is_empty() || !keyEvents.ContainsModifier(&KeyModifiers::Option) {
                self.codeTabs.tabs[self.lastTab]
                    .InsertChars("    ".to_string(), &self.luaSyntaxHighlightScripts, rustAnalyzer).await;
            } else {
                self.codeTabs.tabs[self.lastTab]
                    .RemoveCurrentToken_NonUpdate();
                self.codeTabs.tabs[self.lastTab]
                    .InsertChars(self.suggested.clone(), &self.luaSyntaxHighlightScripts, rustAnalyzer).await;
            }
        }
    }

    async fn CutCode<'b> (&mut self,
                          _keyEvents: &KeyParser,
                          clipBoard: &mut Clipboard,
                          rustAnalyzer: RustAnalyzerLsp<'b>
    ) {
        // get the highlighted section of text.... or the line if none
        let tab = &mut self.codeTabs.tabs[self.lastTab];
        let text = tab.GetSelection();
        let _ = clipBoard.set_text(text);

        // clearing the rest of the selection
        if tab.highlighting {
            tab.DelChars(0, 0, &self.luaSyntaxHighlightScripts, rustAnalyzer).await;
        } else {
            tab.lines[tab.cursor.0].clear();
            tab.RecalcTokens(tab.cursor.0, 0, &self.luaSyntaxHighlightScripts).await;
            tab.CreateScopeThread(tab.cursor.0, tab.cursor.0, rustAnalyzer);
            //(tab.scopes, tab.scopeJumps, tab.linearScopes) = GenerateScopes(&tab.lineTokens, &tab.lineTokenFlags, &mut tab.outlineKeywords);
        }
    }

    async fn PasteCodeLoop<'b> (&mut self,
                                splitLength: usize,
                                i: usize,
                                rustAnalyzer: RustAnalyzerLsp<'b>
    ) {
        let tab = &mut self.codeTabs.tabs[self.lastTab];

        if i < splitLength {
            // why does highlight need to be set to true?????? This makes noooo sense??? I give up
            tab.LineBreakIn(true, &self.luaSyntaxHighlightScripts, rustAnalyzer).await;
            // making sure all actions occur on the same iteration
        }
        if i >0 && i < splitLength {
            if let Some(mut elements) = tab.changeBuffer.pop() {
                while let Some(element) = elements.pop() {
                    let size = tab.changeBuffer.len() - 1;
                    tab.changeBuffer[size].insert(0, element);
                }
            }
        }
    }

    async fn PasteCode<'b> (&mut self,
                            _keyEvents: &KeyParser,
                            clipBoard: &mut Clipboard,
                            rustAnalyzer: RustAnalyzerLsp<'b>
    ) {
        // pasting in the text
        if let Ok(text) = clipBoard.get_text() {
            let splitText = text.split('\n');
            let splitLength = splitText.clone().count() - 1;
            for (i, line) in splitText.enumerate() {
                if line.is_empty() {  continue;  }
                self.codeTabs.tabs[self.lastTab].InsertChars(
                    line.to_string(), &self.luaSyntaxHighlightScripts,
                    rustAnalyzer
                ).await;
                self.PasteCodeLoop(splitLength, i, rustAnalyzer).await;
            }
        }
    }

    fn FindCodeReferenceLine (&mut self, _keyEvents: &KeyParser, _clipBoard: &mut Clipboard) {
        // finding the nearest occurrence to the cursor
        let tab = &mut self.codeTabs.tabs[self.lastTab];
        if tab.highlighting {
            // getting the new search term (yk, it's kinda easy when done the right way the first time......not happening again though)
            let selection = tab.GetSelection();
            tab.searchTerm = selection;
        }

        // searching for the term
        let mut lastDst = (usize::MAX, 0usize);
        for (index, line) in tab.lines.iter().enumerate() {
            let dst = (index as isize - tab.cursor.0 as isize).saturating_abs() as usize;
            if !line.contains(&tab.searchTerm) {  continue;  }
            if dst > lastDst.0 {  break;  }
            lastDst = (dst, index);
        }

        if lastDst.0 < usize::MAX {
            tab.searchIndex = lastDst.1;
        }
    }

    async fn HandleUndoRedoCode<'b> (&mut self,
                                     keyEvents: &KeyParser,
                                     _clipBoard: &mut Clipboard,
                                     rustAnalyzer: RustAnalyzerLsp<'b>
    ) {
        if keyEvents.ContainsModifier(&KeyModifiers::Shift) ||
            keyEvents.charEvents.contains(&'r') {  // common/control + r | z+shift = redo
            self.codeTabs.tabs[self.lastTab].Redo(&self.luaSyntaxHighlightScripts, rustAnalyzer).await;
        } else {
            self.codeTabs.tabs[self.lastTab].Undo(&self.luaSyntaxHighlightScripts, rustAnalyzer).await;
        }
    }

    async fn HandleCodeCommands<'b> (&mut self,
                                     keyEvents: &KeyParser,
                                     clipBoard: &mut Clipboard,
                                     rustAnalyzer: RustAnalyzerLsp<'b>
    ) {
        if keyEvents.ContainsModifier(&self.preferredCommandKeybind) &&
            keyEvents.ContainsChar('s')
        {
            // saving the program
            self.codeTabs.tabs[self.lastTab].Save();
        } else if keyEvents.ContainsModifier(&self.preferredCommandKeybind) &&
            keyEvents.charEvents.contains(&'f')
        {
            self.FindCodeReferenceLine(keyEvents, clipBoard);
        } else if keyEvents.ContainsModifier(&self.preferredCommandKeybind) &&
            (keyEvents.charEvents.contains(&'z') ||  // command z = undo/redo
                keyEvents.charEvents.contains(&'u') ||  // control/command u = undo
                keyEvents.charEvents.contains(&'r'))  // control/command + r = redo
        {
            self.HandleUndoRedoCode(keyEvents, clipBoard, rustAnalyzer).await;
        } else if keyEvents.ContainsModifier(&self.preferredCommandKeybind) &&
            keyEvents.charEvents.contains(&'c')
        {
            // get the highlighted section of text.... or the line if none
            let text = self.codeTabs.tabs[self.lastTab].GetSelection();
            let _ = clipBoard.set_text(text);
        } else if keyEvents.ContainsModifier(&self.preferredCommandKeybind) &&
            keyEvents.charEvents.contains(&'x')
        {
            self.CutCode(keyEvents, clipBoard, rustAnalyzer).await;
        } else if keyEvents.ContainsModifier(&self.preferredCommandKeybind) &&
            keyEvents.charEvents.contains(&'v')
        {
            self.PasteCode(keyEvents, clipBoard, rustAnalyzer).await;
        }
    }

    async fn HandleCodeKeyEvents<'b> (&mut self,
                                      keyEvents: &KeyParser,
                                      clipBoard: &mut Clipboard,
                                      rustAnalyzer: RustAnalyzerLsp<'b>
    ) {
        self.TypeCode(keyEvents, clipBoard, rustAnalyzer).await;

        if keyEvents.ContainsKeyCode(KeyCode::Delete) {
            self.DeleteCode(keyEvents, clipBoard, rustAnalyzer).await;
        } else if keyEvents.ContainsChar('w') &&
            keyEvents.ContainsModifier(&KeyModifiers::Option)
        {
            self.CloseCodePane();
        } else if keyEvents.ContainsKeyCode(KeyCode::Left) {
            self.MoveCodeCursorLeft(keyEvents, clipBoard);
        } else if keyEvents.ContainsKeyCode(KeyCode::Right) {
            self.MoveCodeCursorRight(keyEvents, clipBoard);
        } else if keyEvents.ContainsKeyCode(KeyCode::Up) {
            self.MoveCodeCursorUp(keyEvents, clipBoard);
        } else if keyEvents.ContainsKeyCode(KeyCode::Down) {
            self.MoveCodeCursorDown(keyEvents, clipBoard);
        } else if keyEvents.ContainsKeyCode(KeyCode::Tab) {
            self.HandleCodeTabPress(keyEvents, clipBoard, rustAnalyzer).await;
        } else if keyEvents.ContainsKeyCode(KeyCode::Return) {
            self.codeTabs.tabs[self.lastTab]
                .LineBreakIn(false, &self.luaSyntaxHighlightScripts, rustAnalyzer).await;  // can't be highlighting if breaking?
        } else if
            keyEvents.charEvents.contains(&'a') &&
            keyEvents.ContainsModifier(&KeyModifiers::Control)
        {
            let tab = &mut self.codeTabs.tabs[self.lastTab];
            let newCursor = (tab.lines.len() - 1, tab.lines[tab.lines.len() - 1].len());
            let difference = newCursor.0 - tab.cursor.0;
            tab.mouseScrolledFlt -= difference as f64;
            tab.mouseScrolled -= difference as isize;
            tab.cursorEnd = (0, 0);
            tab.cursor = newCursor;
            tab.highlighting = true;
        } else {
            self.HandleCodeCommands(keyEvents, clipBoard, rustAnalyzer).await;
        }
    }

    fn HandleMenuKeyEvents (&mut self, keyEvents: &KeyParser) {
        if !self.currentCommand.is_empty() {
            self.HandleMenuCommandKeyEvents(keyEvents);
        }

        if self.menuState == MenuState::Settings {
            self.HandleSettingsKeyEvents(keyEvents);
        }

        if keyEvents.ContainsModifier(&KeyModifiers::Option) {  return;  }

        for chr in &keyEvents.charEvents {
            self.currentCommand.push(*chr);

            if self.currentCommand.starts_with("open ") && *chr == '/' {
                // recalculating the directories
                self.currentDir = FileBrowser::GetPathName(
                    self.currentCommand.get(5..)
                        .unwrap_or("")
                );

                self.dirFiles.clear();
                FileBrowser::CalculateDirectories(&self.currentDir, &mut self.dirFiles);
            }
        }
    }

    fn HandleMenuCommandKeyEvents (&mut self, keyEvents: &KeyParser) {
        // quiting
        if keyEvents.ContainsKeyCode(KeyCode::Return) {
            match self.currentCommand.as_str() {
                "q" => {  self.Exit();  },
                "settings" => {  self.menuState = MenuState::Settings;  }
                _ if self.currentCommand.starts_with("open ") => {
                    let foundFile = self.fileBrowser
                        .LoadFilePath(self.currentCommand
                                          .get(5..)
                                          .unwrap_or(""), &mut self.codeTabs);
                    if foundFile.is_ok() {
                        self.fileBrowser.fileCursor = 0;
                        self.codeTabs.currentTab = 0;

                        self.appState = AppState::CommandPrompt;

                        self.allFiles.clear(); self.RecalcAllFiles();
                    }
                },
                "-light" => {  TermRender::ColorMode::ToLight();  },
                "-dark" => {  TermRender::ColorMode::ToDark();  },
                _ => {}
            }

            self.currentCommand.clear();
        } else if keyEvents.ContainsKeyCode(KeyCode::Delete) {
            self.currentCommand.pop();
        } else if keyEvents.ContainsKeyCode(KeyCode::Tab) &&
            self.currentCommand.starts_with("open ")
        {
            self.OpenCodeProject();
        }
    }

    fn OpenCodeProject(&mut self) {
        // handling suggested directory auto fills
        let currentToken = self.currentCommand
            .split("/")
            .last()
            .unwrap_or("");

        for path in &self.dirFiles {
            if path.starts_with(currentToken) {
                let pathEnding = path
                    .get(currentToken.len()..)
                    .unwrap_or("")
                    .to_string();
                self.currentCommand.push_str(&pathEnding);
                self.currentCommand.push('/');

                // recalculating the directories
                self.currentDir = FileBrowser::GetPathName(
                    self.currentCommand.get(5..)
                        .unwrap_or("")
                );

                self.dirFiles.clear();
                FileBrowser::CalculateDirectories(&self.currentDir, &mut self.dirFiles);
                break;
            }
        }
    }

    fn HandleSettingsLeft (&mut self, _keyEvents: &KeyParser) {
        match self.currentMenuSettingBox {
            0 => {
                self.colorMode.colorType = match self.colorMode.colorType {
                    ColorTypes::Basic => ColorTypes::Basic,
                    ColorTypes::Partial => ColorTypes::Basic,
                    ColorTypes::True => ColorTypes::Partial,
                }
            },
            1 => {
                if self.preferredCommandKeybind == KeyModifiers::Control {
                    self.preferredCommandKeybind = KeyModifiers::Command;
                }
            }
            _ => {},
        }
    }

    fn HandleSettingsRight (&mut self, _keyEvents: &KeyParser) {
        match self.currentMenuSettingBox {
            0 => {
                self.colorMode.colorType = match self.colorMode.colorType {
                    ColorTypes::Basic => ColorTypes::Partial,
                    ColorTypes::Partial => ColorTypes::True,
                    ColorTypes::True => ColorTypes::True,
                }
            },
            1 => {
                if self.preferredCommandKeybind == KeyModifiers::Command {
                    self.preferredCommandKeybind = KeyModifiers::Control;
                }
            }
            _ => {},
        }
    }

    fn HandleSettingsKeyEvents (&mut self, keyEvents: &KeyParser) {
        if keyEvents.ContainsKeyCode(KeyCode::Left) {
            self.HandleSettingsLeft(keyEvents);
        } else if keyEvents.ContainsKeyCode(KeyCode::Right) {
            self.HandleSettingsRight(keyEvents);
        } else if keyEvents.ContainsKeyCode(KeyCode::Up) {
            self.currentMenuSettingBox = self.currentMenuSettingBox.saturating_sub(1);
        } else if keyEvents.ContainsKeyCode(KeyCode::Down) {
            self.currentMenuSettingBox += 1;
        }
    }

    async fn HandleKeyEvents<'b> (&mut self,
                                  keyEvents: &KeyParser,
                                  clipBoard: &mut Clipboard,
                                  rustAnalyzer: RustAnalyzerLsp<'b>
    ) {
        match self.appState {
            AppState::CommandPrompt => {
                self.HandleCommandPromptKeyEvents(keyEvents);
            },
            AppState::Tabs => {
                if !self.codeTabs.tabs.is_empty() {
                    self.HandleCodeKeyEvents(keyEvents, clipBoard, rustAnalyzer).await;
                }
                if self.tabState != TabState::Code {
                    self.tabState = TabState::Code
                }
            },
            AppState::Menu => {
                self.HandleMenuKeyEvents(keyEvents);
            },
            //_ => {},
        }
        
        // handling escape (switching tabs)
        if keyEvents.ContainsKeyCode(KeyCode::Escape) &&
            self.fileBrowser.fileOptions.selectedOptionsTab == OptionTabs::Null
        {
            self.appState = match self.appState {
                AppState::Tabs if !self.codeTabs.tabs.is_empty() => {
                    self.tabState = TabState::Files;

                    AppState::CommandPrompt
                },
                AppState::CommandPrompt if !self.codeTabs.tabs.is_empty() => {
                    if matches!(self.tabState, TabState::Files | TabState::Tabs) {
                        self.tabState = TabState::Code;
                    }

                    AppState::Tabs
                },
                AppState::Menu => {
                    if self.menuState == MenuState::Settings {
                        self.menuState = MenuState::Welcome;
                    }
                    AppState::Menu
                },
                _ => {  self.appState.clone()  },
            }
        }
    }

    fn Exit(&mut self) {
        *self.exit.write() = true;
    }

    // ============================================= file block here =============================================
    fn RenderFileBlock (&mut self, app: &mut TermRender::App){
        let coloredTabText = self.codeTabs.GetColoredNames(
            self.appState == AppState::CommandPrompt && self.tabState == TabState::Tabs
        );
        let tabText = vec![
            Span::FromTokens(coloredTabText)
        ];

        {
            let window = app.GetWindowReferenceMut(String::from("Tabs"));
            window.TryUpdateLines(tabText);
        }
    }

    // ============================================= code block here =============================================
    fn RenderCodeBlock (&mut self, app: &mut TermRender::App) {
        if self.codeTabs.tabs.is_empty() {  return;  }

        let leftPadding =
            if self.appState == AppState::CommandPrompt {  29  }
            else {  0  };
        let tabSize = self.codeTabs.GetTabSize(app.GetWindowArea(), leftPadding);

        for tabIndex in 0..=self.codeTabs.panes.len() {
            //let area = app.GetWindowArea();
            self.RenderCodeTab(app, tabIndex, tabSize);
        }

        // rendering the info on the cursor's position
        let tab = &self.codeTabs.tabs[self.lastTab];
        let text = Span::FromTokens(vec![
            if tab.highlighting {
                let (start, end) = (
                    std::cmp::min(tab.cursor.0, tab.cursorEnd.0),
                    std::cmp::max(tab.cursor.0, tab.cursorEnd.0)
                );
                color![format!("Line: {} - {} ({} lines)", start, end, end - start + 1), BrightBlack, Italic]
            } else {
                let charCount = tab.lines[tab.cursor.0].chars().count();
                let charCursor = std::cmp::min(tab.cursor.1 + 1, charCount);
                color![format!("Line: {}/{} ({}%)   Char: {}/{} ({}%)",
                        tab.cursor.0 + 1,
                        tab.lines.len(),
                        ((tab.cursor.0 as f64 + 1.0) / tab.lines.len() as f64 * 100.0) as usize,
                        charCursor,
                        charCount,
                        (charCursor as f64 / charCount as f64 * 100.0) as usize
                ), BrightBlack, Italic]
        }]);
        let window = app.GetWindowReferenceMut(String::from("CursorInfo"));
        window.TryUpdateLines(vec![text]);
    }

    fn RenderCodeTab(&mut self, app: &mut TermRender::App, tabIndex: usize, tabSize: usize) {
        let trueTabIndex = if tabIndex == 0 { self.codeTabs.currentTab } else { self.codeTabs.panes[tabIndex - 1] };
        let name = self.codeTabs.tabs[trueTabIndex].name.clone();
        let codeBlockTitle = Span::FromTokens(vec![
            color![" ", BrightWhite],
            color![name, Bold],
            color![" ", BrightWhite],
        ]);

        let padding: u16 =
            if self.appState == AppState::CommandPrompt {  30  }
            else {  0  };

        let mut rect = app.GetWindowArea().clone();
        rect.width = tabSize as u16 + (padding.saturating_sub(1));
        let codeText =
            self.codeTabs.GetScrolledText(
                &rect,
                self.appState == AppState::Tabs &&
                    self.tabState == TabState::Code,
                &self.colorMode,
                &self.suggested,
                trueTabIndex,
                padding.saturating_sub(1),
        );

        let height = self.area.height;
        let window = app.GetWindowReferenceMut(format!("CodeBlock{name}"));
        if name != self.lastTabName && self.codeTabs.panes.is_empty() {  window.UpdateAll();  }
        self.lastTabName = name;

        // updating the sizing (incase it was changed to a pane)
        window.Move((
            (tabIndex * tabSize) as u16 + padding, 2
        ));
        if window.Resize((
            tabSize as u16,
            height - 9
        )) {
            self.codeTabs.tabs[trueTabIndex].ClearRenderCache();
        }

        if self.appState == AppState::CommandPrompt && self.tabState == TabState::Code {
            window.TryColorize(ColorType::BrightBlue);
        } else {
            window.ClearColors();
        }

        if !window.HasTitle() {
            window.TitledColored(codeBlockTitle);
        }

        window.TryUpdateLines(codeText);
        //window.FromLines(codeText);
    }

    // this method is a mess..... but it works so whatever
    fn HandleKeywordSelf (&mut self, currentScope: &mut Vec <usize>) {
        let mut scope = currentScope.clone();
        scope.reverse();
        // this drops after this scope; this is the main thread so I don't really care
        let node = self.codeTabs.tabs[self.lastTab].scopes.read();
        if scope.len() < 2 || node.children.len() >= scope[1] {  return;  }
        let baseNode = node.GetNode(&mut vec![scope[1]]);
        let implLine = baseNode.start;
        // searching for the container which has this as an impl line
        for keyword in self.codeTabs.tabs[self.lastTab].outlineKeywords.read().iter() {
            if keyword.implLines.contains(&implLine) {
                currentScope.clear();
                for scope in &self.codeTabs.tabs[self.lastTab].scopeJumps.read()[keyword.lineNumber] {
                    currentScope.push(*scope);
                }
                break;
            }
        }
    }

    // error, not working todo; fix
    fn HandleKeywordScopes (&mut self, baseToken: String, currentScope: &mut Vec <usize>) {
        if &baseToken == "self" {
            self.HandleKeywordSelf (currentScope);
            return;
        }
        let baseKeywords = OutlineKeyword::TryFindKeywords(
            &std::sync::Arc::new(parking_lot::RwLock::new(OutlineKeyword::GetValidScoped(
                &self.codeTabs.tabs[self.lastTab].outlineKeywords,
                currentScope
            ))),
            baseToken
        );
        //let size = baseKeywords.len();
        for keyword in baseKeywords {
            // getting the base type
            if keyword.typedType.is_none() {  continue;  }
            let newKeywordBase = OutlineKeyword::TryFindKeywords(
                &self.codeTabs.tabs[self.lastTab].outlineKeywords,
                keyword.typedType.clone().unwrap_or(String::new())
            );
            /*for newKeyword in newKeywordBase*/ {
                //self.debugInfo.push_str(&format!("{:?}; {:?}", keyword.typedType, newKeyword.childKeywords));
                // figure out how to add the children to suggestions
                if newKeywordBase.is_empty() {  continue;  }
                *currentScope = self.codeTabs.tabs[self.lastTab].scopeJumps.read()[
                    newKeywordBase[0].lineNumber
                ].clone();
                //break;
            }
        }
    }

    fn CalculateInnerScope (&mut self, currentScope: &mut Vec <usize>, tokenSet: &mut Vec <String>) {
        if tokenSet.is_empty() {  return;  }

        let baseToken = tokenSet[tokenSet.len() - 1].clone();
        let mut currentElement = OutlineKeyword::TryFindKeyword(
            &self.codeTabs.tabs[self.lastTab].outlineKeywords,
            tokenSet.pop().unwrap_or_default(),
        );
        if let Some(set) = &currentElement {
            let newScope = self.codeTabs.tabs[self.lastTab].scopeJumps.read()[
                set.lineNumber
                ].clone();
            *currentScope = newScope;

            if set.childKeywords.is_empty() {
                self.HandleKeywordScopes(baseToken, currentScope);
                return;
            }
        } else if baseToken == "self" {
            self.HandleKeywordScopes(baseToken, currentScope);
        }

        while !tokenSet.is_empty() && currentElement.is_some() {
            //self.debugInfo.push(' ');
            let newToken = tokenSet.remove(0);
            if let Some(set) = currentElement {
                let newScope = self.codeTabs.tabs[self.lastTab].scopeJumps.read()[
                    set.lineNumber
                    ].clone();
                //self.debugInfo.push_str(&format!("{:?} ", newScope.clone()));
                *currentScope = newScope;
                currentElement = OutlineKeyword::TryFindKeyword(
                    &std::sync::Arc::new(parking_lot::RwLock::new(set.childKeywords)),
                    newToken
                );
            }
        }
    }

    fn ParseKeywordSuggestions (&mut self, validKeywords: Vec <OutlineKeyword>, token: String) {
        if matches!(token.as_str(), " " | "," | "|" | "}" | "{" | "[" | "]" | "(" | ")" |
                    "+" | "=" | "-" | "_" | "!" | "?" | "/" | "<" | ">" | "*" | "&" |
                    "." | ";")
            {  return;  }

        let mut closest = (usize::MAX, vec!["".to_string()], 0usize);
        for (i, var) in validKeywords.iter().enumerate() {
            let value = string_pattern_matching::byte_comparison(&token, &var.keyword);
            // StringPatternMatching::levenshtein_distance(&token, &var.keyword); (too slow)
            if value < closest.0 {
                closest = (value, vec![var.keyword.clone()], i);
            } else if value == closest.0 {
                closest.1.push(var.keyword.clone());
            }
        }
        // getting the closest option to the size of the current token if there are multiple equal options
        let mut finalBest = (usize::MAX, "".to_string(), 0usize);
        for element in closest.1 {
            let size = (element.len() as isize - token.len() as isize).unsigned_abs();
            if size < finalBest.0 {
                finalBest = (size, element, closest.2);
            }
        }

        if closest.0 < 15 {  // finalBest.1 != token.as_str()
            self.suggested = finalBest.1;
        }
    }

    fn UpdateRenderErrorBar (&mut self) {
        if self.codeTabs.tabs.is_empty() {  return;  }

        self.suggested.clear();
        //let mut scope = self.codeTabs.tabs[self.lastTab].scopeJumps[
        //    self.codeTabs.tabs[self.lastTab].cursor.0
        //    ].clone();
        let mut tokenSet: Vec<String> = vec!();
        self.codeTabs.tabs[self.lastTab].GetCurrentToken(&mut tokenSet);
        if tokenSet.is_empty() {  return;  }  // token set is correct it seems

        let token = tokenSet.remove(0);  // getting the item actively on the cursor
        if self.codeTabs.tabs[self.lastTab].scopeJumps.read().len() <= self.codeTabs.tabs[self.lastTab].cursor.0 {  return;  }
        let mut currentScope =
            self.codeTabs.tabs[self.lastTab].scopeJumps.read()[
                self.codeTabs.tabs[self.lastTab].cursor.0
            ].clone();
        self.CalculateInnerScope(&mut currentScope, &mut tokenSet);

        //scope = currentScope.clone();
        let validKeywords = OutlineKeyword::GetValidScoped(
            &self.codeTabs.tabs[self.lastTab].outlineKeywords,
            &currentScope,
        );
        self.ParseKeywordSuggestions(validKeywords, token);
    }

    // ============================================= Error Bar =============================================
    fn RenderErrorBar (&mut self, app: &mut TermRender::App) {//, area: Rect, buf: &mut Buffer) {
        self.UpdateRenderErrorBar();

        let errorText = vec![
            Span::FromTokens(vec![
                color![format!(": {}", self.suggested), Italic]
                    .Colorize(self.colorMode.colorBindings.suggestion),
            ]),
            Span::FromTokens(vec![
                color![format!("Debug: {}", self.debugInfo), Bold]
                    .Colorize(self.colorMode.colorBindings.errorCol),
                //format!(" ; {:?}", scope).white()
            ]),
        ];

        {
            let window = app.GetWindowReferenceMut(String::from("ErrorBar"));
            window.TryUpdateLines(errorText);
        }
    }

    fn RenderProject (&mut self, app: &mut TermRender::App) {  // (&mut self, area: Rect, buf: &mut Buffer) {
        self.RenderFileBlock(app);
        self.RenderFiles(app);
        self.RenderErrorBar(app);
        self.RenderCodeBlock(app);
    }

    fn RenderSettings (&mut self, app: &mut TermRender::App) {//, area: Rect, buf: &mut Buffer) {
        // ============================================= Color Settings =============================================
        // the color mode setting
        let settingsText = vec![
            Span::FromTokens(vec![
                color!["Color Mode: [", BrightWhite],
                {
                    if self.colorMode.colorType == ColorTypes::Basic {
                        color!["Basic", Yellow, Bold, Underline]
                    } else {
                        color!["Basic", BrightWhite]
                    }
                },
                color!["]", BrightWhite],
                color![" [", BrightWhite],
                {
                    if self.colorMode.colorType == ColorTypes::Partial {
                        color!["8-bit", Yellow, Bold, Underline]
                    } else {
                        color!["8-bit", BrightWhite]
                    }
                },
                color!["]", BrightWhite],
                color![" [", BrightWhite],
                {
                    if self.colorMode.colorType == ColorTypes::True {
                        color!["24-bit", Yellow, Bold, Underline]
                    } else {
                        color!["24-bit", BrightWhite]
                    }
                },
                color!["]", BrightWhite],
            ]),
            Span::FromTokens(vec![
                color![" * Not all terminals accept all color modes. If the colors are messed up, try lowering this",
                    White, Dim, Italic]
            ]),
        ];

        {
            let window = app.GetWindowReferenceMut(String::from("ColorSetting"));
            if self.currentMenuSettingBox == 0 {
                window.TryColorize(ColorType::BrightBlue);
            } else {
                window.ClearColors();
            }
            window.TryUpdateLines(settingsText);
        }

        // ============================================= Key Settings =============================================
        // the color mode setting
        let settingsText = vec![
            Span::FromTokens(vec![
                color!["Preferred Modifier Key: [", BrightWhite],
                {
                    if self.preferredCommandKeybind == KeyModifiers::Command {
                        color!["Command", Yellow, Bold, Underline]
                    } else {
                        color!["Command", BrightWhite]
                    }
                },
                color!["]", BrightWhite],
                color![" [", BrightWhite],
                {
                    if self.preferredCommandKeybind == KeyModifiers::Control {
                        color!["Control", Yellow, Bold, Underline]
                    } else {
                        color!["Control", BrightWhite]
                    }
                },
                color!["]", BrightWhite],
            ]),
            Span::FromTokens(vec![
                color![" * The preferred modifier key for things like ctrl/cmd 'c'", BrightWhite, Dim, Italic]
            ]),
        ];

        {
            let window = app.GetWindowReferenceMut(String::from("KeybindSetting"));
            if self.currentMenuSettingBox == 1 {
                window.TryColorize(ColorType::BrightBlue);
            } else {
                window.ClearColors();
            }
            window.TryUpdateLines(settingsText);
        }
    }

    fn RenderMenu (&mut self, app: &mut TermRender::App) {
        // ============================================= Welcome Text =============================================
        /*

        | welcome!
        |\\            //   .==  ||      _===_    _===_   ||\    /||   .==  ||  |
        | \\          //   ||    ||     //   \\  //   \\  ||\\  //||  //    ||  |
        |  \\  //\\  //    ||--  ||     ||       ||   ||  || \\// ||  ||--      |
        |   \\//  \\//     \\==  ||===  \\___//  \\___//  ||      ||  \\==  []  |

        */

        match self.menuState {
            MenuState::Settings => {
                self.RenderSettings(app);
            },
            MenuState::Welcome => {
                // only updating if the text hasn't been set
                if app.GetWindowReference(String::from("Welcome")).IsEmpty() {
                    let window = app.GetWindowReferenceMut(String::from("Welcome"));

                    let welcomeText = vec![
                        Span::FromTokens(vec![
                            color!["\\\\            //   .==  ||      _===_    _===_   ||\\    /||   .==  ||",
                            Red, Bold],//.red().bold(),
                        ]),
                        Span::FromTokens(vec![
                            color![" \\\\          //   ||    ||     //   \\\\  //   \\\\  ||\\\\  //||  //    ||",
                            Red, Bold],//.red().bold(),
                        ]),
                        Span::FromTokens(vec![
                            color!["  \\\\  //\\\\  //    ||--  ||     ||       ||   ||  || \\\\// ||  ||--    ",
                            Red, Bold],//.red().bold(),
                        ]),
                        Span::FromTokens(vec![
                            color!["   \\\\//  \\\\//     \\\\==  ||===  \\\\__=//  \\\\___//  ||      ||  \\\\==  []",
                            Red, Bold],//.red().bold(),
                        ]),  // 71, 15   35.5
                        Span::FromTokens(vec![]),
                        Span::FromTokens(vec![]),
                        Span::FromTokens(vec![  // 43/2 = 35.5 - 21.5 = 14
                            color!["              The command prompt is bellow (Bottom Left):",
                            White, Bold],//.white().bold()
                        ]),
                        Span::FromTokens(vec![]),
                        Span::FromTokens(vec![
                            color!["                Press: <", BrightWhite, Bold, Dim],//.white().bold().dim(),
                            color!["q", BrightWhite, Bold, Dim, Italic, Underline],//.white().bold().dim().italic().underlined(),
                            color!["> followed by <", BrightWhite, Bold, Dim],//.white().bold().dim(),
                            color!["return", BrightWhite, Bold, Dim, Italic, Underline],//.white().bold().dim().italic().underlined(),
                            color!["> to quit", BrightWhite, Bold, Dim],//.white().bold().dim(),    39/2 = 35.5 - 19.5 = 16
                        ]),
                        Span::FromTokens(vec![
                            color!["           Type ", BrightWhite, Bold, Dim],//.white().bold().dim(),
                            color!["\"open\"", BrightWhite, Bold, Dim, Italic, Underline],//.white().bold().dim().italic().underlined(),
                            color![" followed by the path to the directory", BrightWhite, Bold, Dim],//.white().bold().dim(),
                        ]),  // 49 / 2= 35.5 - 24.5 = 11
                        Span::FromTokens(vec![]),
                        Span::FromTokens(vec![
                            color!["          Type ", BrightWhite, Bold, Dim],//.white().bold().dim(),
                            color!["\"settings\"", BrightWhite, Bold, Dim, Italic, Underline],//.white().bold().dim().underlined().italic(),
                            color![" to open settings ( <", BrightWhite, Bold, Dim],//.white().bold().dim(),
                            color!["esc", BrightWhite, Bold, Dim, Italic, Underline],//.white().bold().dim().italic().underlined(),
                            color!["> to leave )", BrightWhite, Bold, Dim],//.white().bold().dim(),
                        ]),  // 51 / 2 = 35.5 - 25.5 = 10
                        Span::FromTokens(vec![
                            color![" *If you see this, type -light to enter light mode; -dark to return*", Black, Italic],//.white().bold().dim(),
                        ]),
                        //Line::from(vec![
                        //    self.dirFiles.concat().white().bold().dim(),
                        //]),
                    ];

                    window.TryUpdateLines(welcomeText);
                }
            }
        }
    }

    fn CheckWindowsProject (&mut self, app: &mut TermRender::App) {
        if app.ContainsWindow(String::from("Tabs")) {
            let window = app.GetWindowReferenceMut(String::from("Tabs"));
            window.Move((30, 0));
            window.Resize((self.area.width - 29, 1));
        } else {
            let window = TermRender::Window::new(
                (30, 0), 0,
                (self.area.width - 29, 1),
            );
            //window.Bordered();
            app.AddWindow(window, String::from("Tabs"), vec![String::from("Project")]);
        }

        // file, tabs, error thingy, code
        if app.ContainsWindow(String::from("ErrorBar")) {
            let window = app.GetWindowReferenceMut(String::from("ErrorBar"));
            window.Move((
                0, self.area.height - 8,
            ));
            window.Resize((self.area.width, 8));
        } else {
            let mut window = TermRender::Window::new(
                (0, self.area.height - 8), 0,
                (self.area.width, 8)
            );
            window.Bordered();
            app.AddWindow(window, String::from("ErrorBar"), vec![String::from("Project")]);
        }

        if app.ContainsWindow(String::from("Files")) {
            let window = app.GetWindowReferenceMut(String::from("Files"));
            window.Move((
                0, 1,
            ));
            window.Resize((30, self.area.height - 8));
        } else {
            let mut window = TermRender::Window::new(
                (0, 1), 0,
                (30, self.area.height - 8)
            );
            window.Bordered();
            app.AddWindow(window, String::from("Files"), vec![String::from("Project")]);
        }

        if app.ContainsWindow(String::from("FileOptions")) {
            let window = app.GetWindowReferenceMut(String::from("FileOptions"));
            window.Move((1, 0));
            window.Resize((30, 1));
        } else {
            let window = TermRender::Window::new(
                (1, 0), 1,
                (30, 1)
            );
            app.AddWindow(window, String::from("FileOptions"), vec![String::from("FileOptions")]);
        }

        //*
        if app.ContainsWindow(String::from("FileOptionsDrop")) {
            let window = app.GetWindowReferenceMut(String::from("FileOptionsDrop"));
            window.Move((1, 2));
            window.Resize((15, 9));
        } else {
            let mut window = TermRender::Window::new(
                (1, 2), 1,
                (15, 9)
            );
            window.Bordered();
            window.Colorize(ColorType::OnBrightBlack);
            window.Hide();  // the error is somewhere in all of this :((((
            window.SupressUpdates();
            app.AddWindow(window, String::from("FileOptionsDrop"), vec![
                String::from("FileOptions"),
                String::from("FileDropDown"),
                String::from("FileDrop1")
            ]);
        }  // */

        // dealing with the annoying code tabs
        self.CheckCodeTabs(app, (self.area.width, self.area.height));

        if app.ChangedWindowLayout() {
            app.PruneByKeywords(
                vec![String::from("Menu")]
            );
        }
    }

    // prunes any code tabs that are no longer open (there was a small, probably never noticable memory leak before)
    fn PruneBadCodeTabs (&mut self, app: &mut TermRender::App, toPrune: Vec <String>, names: &mut Vec <String>) {
        for windowName in toPrune {
            for i in 0..names.len() {
                if names[i] == windowName {  // making sure no windows are left behind
                    names.remove(i);
                }
            }
            let _ = app.RemoveWindow(windowName);
        }
    }

    fn CheckCodeTabs (&mut self, app: &mut TermRender::App, terminalSize: (u16, u16)) {
        let mut names = app.GetWindowsByKeywordsNonRef(vec![
            String::from("CodeTab")
        ]);  // current active tabs
        let mut toPrune = vec![];
        for name in &mut names {
            let size = name.len();
            let newName = name[9..size].to_string();
            let mut valid = false;
            for tab in &self.codeTabs.tabs {
                if tab.name == *newName {
                    valid = true;
                    break;
                }
            } if !valid {  toPrune.push(name.clone());  }
            *name = newName;
        }

        self.PruneBadCodeTabs(app, toPrune, &mut names);

        let (padding, shift);
        if self.appState == AppState::CommandPrompt {
            let changed = app.GetWindowReferenceMut(String::from("Files")).Show();
            if changed {  app.GetWindowReferenceMut(String::from("FileOptions")).UpdateAll();  }
            (padding, shift) = (30, 29);
        } else {
            let changed = app.GetWindowReferenceMut(String::from("Files")).Hide();
            if changed {  app.GetWindowReferenceMut(String::from("FileOptions")).UpdateAll();  }
            (padding, shift) = (0, 0);
        }

        self.UpdateCodeTabsWindows(app, terminalSize, names, padding, shift);
    }

    fn UpdateCodeTabsWindows(&mut self,
                             app: &mut TermRender::App,
                             terminalSize: (u16, u16),
                             names: Vec <String>,
                             padding: u16,
                             shift: u16
    ) {
        // going through all windows and making sure that window exists
        // deleting windows that are no longer open
        for tab in &self.codeTabs.tabs {
            if names.contains(&tab.name) {  continue;  }

            // creating a new window
            let mut window = TermRender::Window::new(
                (padding, 2), 0,
                ((terminalSize.0 - shift) / (self.codeTabs.panes.len() as u16 + 1), terminalSize.1 - 9),
            );
            window.Bordered();
            app.AddWindow(window,
                          format!("CodeBlock{}", tab.name),
                          vec![String::from("CodeTab"),
                               tab.name.clone()]
            );
        }

        // the cursor information bar
        if app.ContainsWindow(String::from("CursorInfo")) {
            let window = app.GetWindowReferenceMut(String::from("CursorInfo"));
            window.Move((self.area.width - 50, self.area.height));
            window.Resize((50, 1));
        } else {
            let window = TermRender::Window::new(
                (self.area.width - 50, self.area.height),
                1, (50, 1)
            );
            app.AddWindow(window, String::from("CursorInfo"), vec![String::from("Cursor")]);
        }
    }

    fn CheckWindowsMenu (&mut self, app: &mut TermRender::App) {
        // settings, info text, whatever else
        match self.menuState {
            MenuState::Welcome => {
                if app.ContainsWindow(String::from("Welcome")) {
                    let window = app.GetWindowReferenceMut(String::from("Welcome"));
                    window.Move((
                        self.area.width / 2 - 71/2,
                        self.area.height / 2 - 10,
                    ));
                    window.Resize((71, 15));
                } else {
                    let mut window = TermRender::Window::new(
                        (self.area.width / 2 - 71/2, self.area.height / 2 - 10),
                        0, (71, 15)
                    );
                    window.Bordered();
                    app.AddWindow(window, String::from("Welcome"), vec![String::from("Menu")]);
                    //app.UpdateWindowLayoutOrder();  // resized windows will be moved but still ordered the same
                }

                if app.ChangedWindowLayout() {
                    let _ = app.PruneByKeywords(vec![String::from("Settings")]);
                    //self.currentCommand = String::from("pruning");
                }
            },
            MenuState::Settings => {
                if app.ContainsWindow(String::from("ColorSetting")) {
                    let window = app.GetWindowReferenceMut(String::from("ColorSetting"));
                    window.Move((
                        10, 2,
                    ));
                    window.Resize((self.area.width - 20, 4));
                } else {
                    let mut window = TermRender::Window::new(
                        (10, 2), 0,
                        (self.area.width - 20, 4)
                    );
                    window.Bordered();
                    app.AddWindow(window, String::from("ColorSetting"), vec![
                        String::from("Menu"), String::from("Settings")
                    ]);
                    //app.UpdateWindowLayoutOrder();  // resized windows will be moved but still ordered the same
                }
                if app.ContainsWindow(String::from("KeybindSetting")) {
                    let window = app.GetWindowReferenceMut(String::from("KeybindSetting"));
                    window.Move((
                        10, 6,
                    ));
                    window.Resize((self.area.width - 20, 4));
                } else {
                    let mut window = TermRender::Window::new(
                        (10, 6), 0,
                        (self.area.width - 20, 4)
                    );
                    window.Bordered();
                    app.AddWindow(window, String::from("KeybindSetting"), vec![
                        String::from("Menu"), String::from("Settings")
                    ]);
                    //app.UpdateWindowLayoutOrder();  // resized windows will be moved but still ordered the same
                }

                if app.ChangedWindowLayout() {
                    let _ = app.PruneByKey(Box::new(|keywords| {
                        keywords.contains(&String::from("Menu")) &&
                            !keywords.contains(&String::from("Settings"))
                    }));
                    //self.currentCommand = String::from("pruning");
                }
            },
        };
    }

    fn CheckWindows (&mut self, app: &mut TermRender::App) {
        if app.ContainsWindow(String::from("CommandLine")) {
            let window = app.GetWindowReferenceMut(String::from("CommandLine"));
            window.Move((0, self.area.height));
            window.Resize((self.area.width.saturating_sub(51), 1));
        } else {
            let window = TermRender::Window::new(
                (0, self.area.height), 0,
                (self.area.width.saturating_sub(51), 1),
            );
            app.AddWindow(window, String::from("CommandLine"), vec![]);
            //app.UpdateWindowLayoutOrder();
        }
    }

    fn RenderFrame (&mut self, app: &mut TermRender::App) -> usize {
        self.CheckWindows(app);
        match self.appState {
            AppState::Tabs | AppState::CommandPrompt => {
                self.CheckWindowsProject(app);
                self.RenderProject(app);
            },
            AppState::Menu => {
                self.CheckWindowsMenu(app);
                self.RenderMenu(app)
            }
        }

        // rendering the command line is necessary for all states
        // ============================================= Commandline =============================================
        let commandText =
            Span::FromTokens(vec![
                color!["/", BrightWhite, Bold],//.to_string().white().bold(),
                color![self.currentCommand, BrightWhite, Italic],//.clone().white().italic(),
                {
                    if  self.appState == AppState::Menu &&
                        self.currentCommand.starts_with("open ")
                    {
                        // handling suggested directory auto fills
                        let currentToken = self.currentCommand
                            .split("/")
                            .last()
                            .unwrap_or("");

                        let mut validFinish = String::new();
                        for path in &self.dirFiles {
                            if path.starts_with(currentToken) {
                                validFinish = path
                                    .get(currentToken.len()..)
                                    .unwrap_or("")
                                    .to_string();
                                break;
                            }
                        }

                        color![validFinish, BrightBlack]//.white().dim()
                    } else {
                        color![""]//.white().dim()
                    }
                },
                {
                    if matches!(self.appState, AppState::CommandPrompt | AppState::Menu) {
                        color!["_", BrightWhite, Blink, Bold]//.to_string().white().slow_blink().bold()
                    } else {
                        color![""]//.white()
                    }
                },
        ]);

        let window = app.GetWindowReferenceMut(String::from("CommandLine"));
        window.TryUpdateLines(vec![commandText]);

        // rendering the updated app
        app.Render(Some((
            self.area.width,
            self.area.height,
        )))
    }
}


/*
Commands: √ <esc>
    √ <enter> -> exit commands
    √ <q> + <enter> -> exit application
    √ <tab> -> switch tabs:
    gd -> go to definition (experimental)

        * code editor:
            <cmd> + <1 - 9> -> switch to corresponding code tab

            √ <left/right/up/down> -> movement in the open file
            √ <option> + <left/right> -> jump to next token
            √ <option> + <up/down> -> jump to end/start of current scope
            √ <cmnd> + <left/right> -> jump to start/end of line
                √ - first left jumps to the indented start
                √ - second left jumps to the true start
            √ <cmnd> + <up/down> -> jump to start/end of file
            √ <shift> + any line movement/jump -> highlight all text selected (including jumps from line # inputs)
            <ctrl> + <[> -> temporarily opens command line to input number of lines to jump up
            <ctrl> + <]> -> temporarily opens command line to input number of lines to jump down
            <ctrl> + <1-9> -> jump up that many positions
            <ctrl> + <1-9> + <shift> -> jump down that many positions

            <ctrl> + <left>/<right> -> open/close scope of code (can be done from any section inside the scope, not just the start/end)

            √ <cmnd> + <c> -> copy (either selection or whole line when none)
            √ <cmnd> + <v> -> paste to current level (align relative indentation to the cursor's)

            √ <del> -> deletes the character the cursor is on
            √ <del> + <option> -> delete the token the cursor is on
            √ <del> + <cmnd> -> delete the entire line
            √ <del> + <shift> + <cmnd/option/none> -> does the same as specified before except to the right instead

            <tab> -> indents the lineup to the predicted indentation
            √ <tab> + <shift> -> unindents the line by one
            √<enter> -> creates a new line
            <enter> + <cmnd> -> creates a new line starting at the end of the current
            <enter> + <cmnd> + <shift> -> creates a new line starting before the current

        * file browser / program outline:
            <up/down> -> move between files/folders
            <enter> -> open in new tab
            <right> -> open settings menu:
                <up/down> -> move between settings
                <enter> -> edit setting
                <left> -> close menu

            √ <shift> + <tab> -> cycle between pg outline and file browser

            outline:
                √ - shows all functions/methods/classes/etc... so they can easily be access without needed the mouse and without wasting time scrolling

                √ <enter> -> jumps the cursor to that section in the code
                <shift> + <enter> -> jumps the cursor to the section and switches to code editing

                √ <down/up> -> moves down or up one
                <option> + <down/up> -> moves up or down to the start/end of the current scope
                <cmnd> + <down>/<up> -> moves to the top or bottom of the outline

                <ctrl> + <left> -> collapse the scope currently selected
                <ctrl> + <right> -> uncollapse the scope

        * code tabs:
            √ <left/right> -> change tab
            √ <option> + <left/right> -> move current tab left/right
            √ <del> -> close current tab

the bottom bar is 4 lines tall (maybe this could be a custom parameter?)
the sidebar appears behind the bottom bar and pops outward shifting the text

Errors underlined in red
Warnings underlined in yellow
    integrate a way to run clippy on the typed code
    parse Clippy's output into the proper warning or errors
    display the error or warning at the bottom (where code completion suggestions go)

Suggestions appear on the very bottom as to not obstruct the code being written (and a basic inline greyed text)


maybe show the outline moving while scrolling?
Add scrolling to the outline
make it so when indenting/unindenting it does it for the entire selection if highlighting
make it so when pressing return, it auto sets to the correct indent level
    same thing for various other actions being set to the correct indent level

Add double-clicking to highlight a token or line
make it so that highlighting than typing () or {} or [] encloses the text rather than replacing it

command f
    Add the ability to type in custom search terms through the command line
    Add the ability to scroll through the various options
    Currently, detection seems to be working at least for single line terms
    Make it jump the cursor to the terms

make command + x properly shift the text onto a new line when pasted (along w/ being at the correct indent level)

make it so that the outline menu, when opened, is placed at the correct location rather than defaulting to the start until re-entering the code tab

maybe move all the checks for the command key modifier to a single check that then corresponds to other checks beneath it? I'm too lazy rn though

Prevent the program from crashing when touch non-u8 characters (either handle the error, or do something but don't crash)
    Ideally the user can still copy and past them along with placing them and deleting them
    (kind of fixed. You can touch it but not interact)

Fix the bug with the scope outline system where it clicks one cell too high when it hasn't been scrolled yet

Fix the bug with highlighting; when highlighting left and pressing right arrow, it stays at the left
    Highlighting right works fine though, idk why

Allow language highlighting to determine unique characters for comments and multi line comments
    More customization through settings, json bindings, or dynamically executed async lua scripts

make a better system that can store keybindings; the user can make a custom one, or there are two defaults: mac custom, standard
    Kind of did this... kinda not

Todo! Add the undo-redo change thingy for the replacing chars stuff when auto-filling

make the syntax highlighting use known variable/function syntax types once it's known (before use the single line context).
make the larger context and recalculation of scopes & specific variables be calculated on a thread based on a queue and joined
    once a channel indicates completion. (maybe run this once every second if a change is detected).

multi-line parameters on functions/methods aren't correctly read
multi-line comments aren't updated properly when just pressing return (on empty lines)
    it may not be storing anything on empty lines?


! whenever loading into a workspace, the lsp initialization seems to cause a brief freeze; figure that out
! it may be due to some .write/.read or i/o limits or something else, idk. It should be all on multiple other threads

*/

fn main() {
    // this runtime is implemented in a way where blocking tasks/blocking thread sleeps don't block others tasks from running
    // each task gets its own thread so blocking is safe unless the section requires a safe/soft exit instead of a hard drop
    let runtime = std::sync::Arc::new(parking_lot::RwLock::new(Runtime::default()));
    let clonedRuntime = runtime.clone();  // creating a clone of the runtime manager (each task gets a unique thread)
    runtime.write().AddTask(Box::pin(async {
        let mut termApp = TermRender::App::new();

        enableMouseCapture().await;
        let _app_result = App::default().Run(&mut termApp, clonedRuntime).await;
        //app_result.unwrap();  // too lazy to make the runtime actually be able to output things....
        // (unwrapping stalls the thread; the result needs to be ignored to allow proper exiting/exit handling)
        disableMouseCapture().await;
    }));
    loop {
        let completed = runtime.write().Poll();
        if completed || runtime.read().is_empty() {  break;  }
        // waiting to reduce cpu usage (this doesn't directly poll the futures; it polls the threads to check if they can be cleaned up)
        // this won't prevent anything from running. It will only cause a slight delay in when threads get joined.
        std::thread::sleep(Duration::from_millis(250));
    }

    // softly shutting down any non-blocked tasks (literally only being used to ensure rust-analyzer
    // is correctly shutdown; otherwise there would be memory leaks to extend well beyond the program's
    // termination)
    runtime.write().SoftShutdown();
}

