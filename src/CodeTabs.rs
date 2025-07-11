use crate::languageServer::{RustAnalyzer, RustEvents};
use crate::TermRender::*;
use crate::TokenInfo::*;
use crate::LuaScripts;
use crate::Tokens::*;
use crate::Colors;
use crate::color;

// the bounds from the screen edge at which the cursor will begin scrolling
const SCROLL_BOUNDS: usize = 12;
const CENTER_BOUNDS: usize = 0;

pub type RustAnalyzerLsp <'a> = &'a Option <std::sync::Arc <parking_lot::RwLock <RustAnalyzer>>>;


pub mod Edits {
    pub type RustAnalyzerLsp <'a> = &'a Option <std::sync::Arc <parking_lot::RwLock <RustAnalyzer>>>;

    use crate::{CodeTab, LuaScripts};
    use crate::languageServer::RustAnalyzer;

    // private sense it's not needed elsewhere (essentially just a modified copy of handleHighlights...)
    async fn RemoveText <'a> (tab: &mut CodeTab,
                              start: (usize, usize),
                              end: (usize, usize),
                              luaSyntaxHighlightScripts: &LuaScripts,
                              rustAnalyzer: RustAnalyzerLsp<'a>,
    ) {
        if end.0 == start.0 {
            tab.lines[end.0].replace_range(end.1..start.1, "");
            tab.RecalcTokens(end.0, 0, luaSyntaxHighlightScripts).await;
        } else {
            tab.lines[end.0].replace_range(end.1.., "");
            tab.RecalcTokens(end.0, 0, luaSyntaxHighlightScripts).await;
            tab.lines[end.0].replace_range(..start.1, "");
            tab.RecalcTokens(start.0, 0, luaSyntaxHighlightScripts).await;
            // go through any inbetween lines and delete them. Also delete one extra line so there aren't to blanks?
            let numBetween = start.0 - end.0 - 1;
            for _ in 0..numBetween {
                tab.lines.remove(end.0 + 1);
                tab.lineTokens.write().remove(end.0 + 1);
                tab.lineTokenFlags.write().remove(end.0 + 1);
            }
            // push the next line onto the first...
            let nextLine = tab.lines[end.0 + 1].clone();
            tab.lines[end.0].push_str(nextLine.as_str());
            tab.RecalcTokens(end.0, 0, luaSyntaxHighlightScripts).await;
            tab.lines.remove(end.0 + 1);
            tab.lineTokens.write().remove(end.0 + 1);
            tab.lineTokenFlags.write().remove(end.0 + 1);
        }
        
        tab.CreateScopeThread(start.0, end.0, rustAnalyzer);
        //(tab.scopes, tab.scopeJumps, tab.linearScopes) = GenerateScopes(&tab.lineTokens, &tab.lineTokenFlags, &mut tab.outlineKeywords);
        tab.cursor = end;
    }

    async fn AddText <'a> (tab: &mut CodeTab,
                           start: (usize, usize),
                           end: (usize, usize),
                           text: &str,
                           luaSyntaxHighlightScripts: &LuaScripts,
                           rustAnalyzer: RustAnalyzerLsp<'a>,
    ) {
        let splitText = text.split('\n');
        //let splitLength = splitText.clone().count() - 1;
        for (i, line) in splitText.enumerate() {
            if line.is_empty() {
                tab.lines.insert(end.0 + i, "".to_string());
                tab.lineTokens.write().insert(end.0 + i, vec![]);
                tab.lineTokenFlags.write().insert(end.0 + i, vec![]);
                tab.RecalcTokens(end.0 + i, 0, luaSyntaxHighlightScripts).await;
                continue;
            }

            if i == 0 {
                if end.1 >= tab.lines[end.0].len() {
                    tab.lines[end.0].push_str(line);
                } else {
                    tab.lines[end.0].insert_str(end.1, line);
                }
                tab.RecalcTokens(end.0, 0, luaSyntaxHighlightScripts).await;
            } else {
                tab.lines.insert(end.0 + i, line.to_string());
                tab.lineTokenFlags.write().insert(end.0 + i, vec![]);
                tab.lineTokens.write().insert(end.0 + i, vec![]);
                tab.RecalcTokens(end.0 + i, 0, luaSyntaxHighlightScripts).await;
            }
        }

        tab.CreateScopeThread(start.0, end.0, rustAnalyzer);
        //(tab.scopes, tab.scopeJumps, tab.linearScopes) = GenerateScopes(&tab.lineTokens, &tab.lineTokenFlags, &mut tab.outlineKeywords);
        tab.cursor = start;
    }
    
    #[derive(Debug)]
    pub struct Deletion {
        pub start: (usize, usize),
        pub end: (usize, usize),  // end is higher/before the start always
        pub text: String,
    }

    impl Deletion {
        pub async fn Undo <'a> (&self,
                                tab: &mut CodeTab,
                                luaSyntaxHighlightScripts: &LuaScripts,
                                rustAnalyzer: RustAnalyzerLsp<'a>,
        ) {
            AddText(tab, self.start, self.end, &self.text, luaSyntaxHighlightScripts, rustAnalyzer).await;
        }
        
        pub async fn Redo <'a> (&self,
                                tab: &mut CodeTab,
                                luaSyntaxHighlightScripts: &LuaScripts,
                                rustAnalyzer: RustAnalyzerLsp<'a>,
        ) {
            RemoveText(tab, self.start, self.end, luaSyntaxHighlightScripts, rustAnalyzer).await
        }
    }
    
    #[derive(Debug)]
    pub struct Addition {
        pub start: (usize, usize),
        pub end: (usize, usize),
        pub text: String,
    }

    impl Addition {
        pub async fn Undo <'a> (&self,
                                tab: &mut CodeTab,
                                luaSyntaxHighlightScripts: &LuaScripts,
                                rustAnalyzer: RustAnalyzerLsp<'a>,
        ) {
            RemoveText(tab, self.start, self.end, luaSyntaxHighlightScripts, rustAnalyzer).await;
        }
        
        pub async fn Redo <'a> (&self,
                                tab: &mut CodeTab,
                                luaSyntaxHighlightScripts: &LuaScripts,
                                rustAnalyzer: RustAnalyzerLsp<'a>,
        ) {
            AddText(tab, self.start, self.end, &self.text, luaSyntaxHighlightScripts, rustAnalyzer).await;
        }
    }

    #[derive(Debug)]
    pub struct NewLine {
        pub position: (usize, usize),
    }

    impl NewLine {
        pub async fn Undo <'a> (&self,
                                tab: &mut CodeTab,
                                luaSyntaxHighlightScripts: &LuaScripts,
                                rustAnalyzer: RustAnalyzerLsp<'a>,
        ) {
            let text = tab.lines.remove(self.position.0 + 1);
            tab.lineTokens.write().remove(self.position.0 + 1);
            tab.lineTokenFlags.write().remove(self.position.0 + 1);
            tab.lines[self.position.0].push_str(text.as_str());
            tab.RecalcTokens(self.position.0, 0, luaSyntaxHighlightScripts).await;

            tab.cursor.0 = self.position.0.saturating_sub(1);
            tab.cursor.1 = tab.lines[tab.cursor.0].len();
            tab.CreateScopeThread(self.position.0, self.position.0, rustAnalyzer);
            //(tab.scopes, tab.scopeJumps, tab.linearScopes) = GenerateScopes(&tab.lineTokens, &tab.lineTokenFlags, &mut tab.outlineKeywords);
        }
        
        pub async fn Redo <'a> (&self,
                                tab: &mut CodeTab,
                                luaSyntaxHighlightScripts: &LuaScripts,
                                rustAnalyzer: RustAnalyzerLsp<'a>,
        ) {
            let rightText = tab.lines[self.position.0]
                .split_off(self.position.1);
            tab.lines.insert(self.position.0 + 1, rightText.to_string());
            tab.lineTokens.write().insert(self.position.0 + 1, vec![]);
            tab.lineTokenFlags.write().insert(self.position.0 + 1, vec![]);
            tab.RecalcTokens(self.position.0 + 1, 0, luaSyntaxHighlightScripts).await;
            tab.RecalcTokens(self.position.0, 0, luaSyntaxHighlightScripts).await;

            tab.cursor = (
                self.position.0 + 1,
                0
            );
            tab.CreateScopeThread(self.position.0, self.position.0 + 1, rustAnalyzer);
            //(tab.scopes, tab.scopeJumps, tab.linearScopes) = GenerateScopes(&tab.lineTokens, &tab.lineTokenFlags, &mut tab.outlineKeywords);
        }
    }

    #[derive(Debug)]
    pub struct RemoveLine {
        pub position: (usize, usize),
    }

    impl RemoveLine {
        pub async fn Undo <'a> (&self,
                                tab: &mut CodeTab,
                                luaSyntaxHighlightScripts: &LuaScripts,
                                rustAnalyzer: RustAnalyzerLsp<'a>,
        ) {
            let rightText = tab.lines[self.position.0]
                .split_off(self.position.1);
            tab.lines.insert(self.position.0 + 1, rightText.to_string());
            tab.lineTokens.write().insert(self.position.0 + 1, vec![]);
            tab.lineTokenFlags.write().insert(self.position.0 + 1, vec![]);
            tab.RecalcTokens(self.position.0 + 1, 0, luaSyntaxHighlightScripts).await;
            tab.RecalcTokens(self.position.0, 0, luaSyntaxHighlightScripts).await;

            tab.cursor = (
                self.position.0 + 1,
                0
            );
            tab.CreateScopeThread(self.position.0, self.position.0 + 1, rustAnalyzer);
            //(tab.scopes, tab.scopeJumps, tab.linearScopes) = GenerateScopes(&tab.lineTokens, &tab.lineTokenFlags, &mut tab.outlineKeywords);
        }
        
        pub async fn Redo <'a> (&self,
                                tab: &mut CodeTab,
                                luaSyntaxHighlightScripts: &LuaScripts,
                                rustAnalyzer: RustAnalyzerLsp<'a>,
        ) {
            let text = tab.lines.remove(self.position.0 + 1);
            tab.lineTokens.write().remove(self.position.0 + 1);
            tab.lineTokenFlags.write().remove(self.position.0 + 1);
            tab.lines[self.position.0].push_str(text.as_str());
            tab.RecalcTokens(self.position.0, 0, luaSyntaxHighlightScripts).await;

            tab.cursor.0 = self.position.0.saturating_sub(1);
            tab.cursor.1 = tab.lines[tab.cursor.0].len();
            tab.CreateScopeThread(self.position.0, self.position.0, rustAnalyzer);
            //(tab.scopes, tab.scopeJumps, tab.linearScopes) = GenerateScopes(&tab.lineTokens, &tab.lineTokenFlags, &mut tab.outlineKeywords);
        }
    }
    
    #[derive(Debug)]
    pub enum Edit {
        Deletion (Deletion),
        Addition (Addition),
        NewLine (NewLine),
        RemoveLine (RemoveLine),
    }
}


// only access the inner value explicitly editing the value, then return ownership
// otherwise the element will block usage of the main item in the codeTab, which
// would block the main thread (aka not good)

// this isn't nearly as long as I thought it would be lol (too lazy to inline it)
type ScopeHandle = std::thread::JoinHandle <()>;

#[derive(Debug)]
pub struct CodeTab {
    pub cursor: (usize, usize),  // line pos, char pos inside line
    pub lines: Vec <String>,
    pub lineTokens: std::sync::Arc <parking_lot::RwLock <Vec <Vec <LuaTuple>>>>,
    // points to the index of the scope (needs adjusting as the tree is modified)
    pub scopeJumps: std::sync::Arc <parking_lot::RwLock <Vec <Vec <usize>>>>,
    pub scopes: std::sync::Arc <parking_lot::RwLock <ScopeNode>>,
    pub linearScopes: std::sync::Arc <parking_lot::RwLock <Vec <Vec <usize>>>>,

    pub scrolled: usize,
    pub mouseScrolled: isize,
    pub mouseScrolledFlt: f64,

    pub name: String,
    pub fileName: String,
    pub cursorEnd: (usize, usize),  // for text highlighting
    pub highlighting: bool,
    pub pauseScroll: u128,

    pub searchIndex: usize,
    pub searchTerm: String,

    pub changeBuffer: Vec <Vec <Edits::Edit>>,
    pub redoneBuffer: Vec <Vec <Edits::Edit>>,  // stores redo's (cleared if undone then edited)
    pub pinedLines: Vec <(usize, ColorType)>,  // todo figure out a way to have a color for the pinned points (maybe an enum?--or just a color....)

    pub outlineKeywords: std::sync::Arc <parking_lot::RwLock <Vec <OutlineKeyword>>>,
    // each line can have multiple flags depending on the depth (each line has a set for each token......)
    pub lineTokenFlags: std::sync::Arc <parking_lot::RwLock <Vec <Vec < Vec <LineTokenFlags>>>>>,

    pub scopeGenerationHandles: Vec <(ScopeHandle, crossbeam::channel::Receiver <bool>)>,

    pub saved: bool,
    pub path: String,

    pub scrollCache: Vec <Span>,
    pub resetCache: Vec <bool>,
    pub shiftCache: i32,
    pub lastScroll: usize,
    pub lastMouse: (usize, usize, usize, usize)
}

impl CodeTab {
    pub fn UpdateScroll (&mut self, acceleration: f64) {
        self.mouseScrolledFlt += acceleration;

        // clamping the bounds
        let lowerBound = -(self.cursor.0 as f64);
        let upperBound = self.lines.len() as f64 - 10.0 - self.cursor.0 as f64;  // 10 lines bellow should be fine?
        self.mouseScrolledFlt =
            f64::min(f64::max(self.mouseScrolledFlt, lowerBound), upperBound);  // lower bound

        self.mouseScrolled = self.mouseScrolledFlt as isize;
    }

    pub fn CreateScopeThread (&mut self,
                              start: usize,
                              end: usize,
                              rustAnalyzer: RustAnalyzerLsp
    ) {
        // offsetting from existing calculations (staggering the computation (1/4 second per thread)
        // if there's at least 2, there should be one still queued and not running
        // so it shouldn't cause outdated information. This value may need adjustment?
        if self.scopeGenerationHandles.len() >= 2 {  return;  }
        // the time of 1/4 seconds is completely random. Seems like it might be reasonable though
        let timeOffset = self.scopeGenerationHandles.len() as u64 * 250;

        // the memory leak is independent of the number of threads or when they're joined (fixed!!!)
        //if !self.scopeGenerationHandles.is_empty() {  return;  }
        let (sender, receiver) = crossbeam::channel::bounded(1);
        let scopeClone = std::sync::Arc::clone(&self.scopes);
        let jumpsClone = std::sync::Arc::clone(&self.scopeJumps);
        let linearClone = std::sync::Arc::clone(&self.linearScopes);

        let lineTokensClone = std::sync::Arc::clone(&self.lineTokens);
        let lineFlagsClone = std::sync::Arc::clone(&self.lineTokenFlags);
        let outlineClone = std::sync::Arc::clone(&self.outlineKeywords);

        let ending = self.fileName.split('.').next_back().unwrap_or("").to_string();

        let rustAnalyzer = rustAnalyzer.clone();
        let charIndex = self.cursor.1;

        self.scopeGenerationHandles.push((
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(timeOffset));

                // put in a request for a lsp update between start and end
                // maybe poll this to make sure it doesn't block anything? idk
                //let completion = false;  // do something with this at some point
                if rustAnalyzer.is_some() {
                    rustAnalyzer.as_ref()
                                .unwrap()
                                .write()
                                .NewEvent(RustEvents::UpdatedLines(start, end, charIndex, String::from(""))
                    );
                }

                let (newScopes, newJumps, newLinear) =
                    Linters::GenerateScopes(&lineTokensClone, &lineFlagsClone, &outlineClone, &ending);

                // waiting for a free spot to let it write (and immediately dropping it)
                let mut writeGuard = scopeClone.write();
                *writeGuard = newScopes;
                // making sure it's not blocking the main thread
                // if something's trying to read, it has to wait for
                // this to be called/finished
                drop(writeGuard);

                // waiting for a free spot to let it write (and immediately dropping it)
                let mut writeGuard = jumpsClone.write();
                *writeGuard = newJumps;
                // making sure it's not blocking the main thread
                // if something's trying to read, it has to wait for
                // this to be called/finished
                drop(writeGuard);

                // waiting for a free spot to let it write (and immediately dropping it)
                let mut writeGuard = linearClone.write();
                *writeGuard = newLinear;
                // making sure it's not blocking the main thread
                // if something's trying to read, it has to wait for
                // this to be called/finished
                drop(writeGuard);

                // sending a message to join the thread
                sender.send(true).unwrap();  // not sure what else to do but throw an error on disconnect
            }),
            receiver
        ));
        //let (thread, rec) = self.scopeGenerationHandles.pop().unwrap();
        //thread.join().unwrap();
    }

    pub fn CheckScopeThreads (&mut self) {
        // the indexes of finished threads so they can be removed and joined
        let mut finishedThreads: Vec <usize> = vec!();

        // checking all thread channels
        for i in 0..self.scopeGenerationHandles.len() {
            let thread = &self.scopeGenerationHandles[i];
            if thread.1.try_recv().is_ok() {
                finishedThreads.push(i);
            }
        }

        // joining necessary threads
        // the indexes are in descending order (ascending order but reversed with .rev)
        // because of this, the indexes shouldn't interfere
        // with the following remove operations
        for index in finishedThreads.into_iter().rev() {
            let thread = self.scopeGenerationHandles.remove(index);
            // ignoring any errors for now
            // all the memory of the thread should be killed
            // all the memory is shared so it should be fine as is
            let _ = thread.0.join();
        }
    }

    pub fn GetCurrentToken (&self, tokenOutput: &mut Vec <String>) {
        let mut accumulate = 0;
        let lineTokensRead = self.lineTokens.read();
        for (tokenIndex, token) in lineTokensRead[self.cursor.0].iter().enumerate() {
            // the cursor can be just right of it, in it, but not just left
            if (accumulate + token.text.len()) < self.cursor.1 || self.cursor.1 <= accumulate {
                accumulate += token.text.len();
                continue;
            }

            tokenOutput.push(token.text.clone());
            for index in (0..tokenIndex).rev() {
                if matches!(lineTokensRead[self.cursor.0][index].text.as_str(),
                    " " | "," | "(" | ")" | ";")
                    {  break;  }
                else if index > 1 &&  // the else reduces the complexity score even though it makes no difference????????????? why?
                    lineTokensRead[self.cursor.0][index].text == ":" &&
                    lineTokensRead[self.cursor.0][index - 1].text == ":"
                {
                    tokenOutput.push(lineTokensRead[self.cursor.0][index - 2].text.clone());
                } else if index > 0 && lineTokensRead[self.cursor.0][index].text == "." {
                    tokenOutput.push(lineTokensRead[self.cursor.0][index - 1].text.clone());
                }
            }
            return;
        } // lineTokensRead is dropped naturally
    }

    // doesn't update the tokens or scopes; requires that to be done elsewhere
    pub fn RemoveCurrentToken_NonUpdate (&mut self) {
        let mut accumulate = 0;
        let lineTokensRead = self.lineTokens.read();
        for token in lineTokensRead[self.cursor.0].iter() {
            // the cursor can be just right of it, in it, but not just left
            if (accumulate + token.text.len()) >= self.cursor.1 && self.cursor.1 > accumulate {
                self.lines[self.cursor.0].replace_range(accumulate..accumulate+token.text.len(), "");
                self.cursor.1 = accumulate;
                return;
            }
            accumulate += token.text.len();
        }  // lineTokensRead is naturally dropped
    }

    pub async fn Undo <'a> (&mut self,
                            luaSyntaxHighlightScripts: &LuaScripts,
                            rustAnalyzer: RustAnalyzerLsp<'a>,
    ) {
        self.scrolled = std::cmp::max(
            self.mouseScrolledFlt as isize + self.scrolled as isize,
            0
        ) as usize;
        self.mouseScrolled = 0;
        self.mouseScrolledFlt = 0.0;
        self.highlighting = false;

        if let Some(edits) = self.changeBuffer.pop() {
            for edit in &edits {
                match edit {
                    Edits::Edit::Addition (action) => {
                        action.Undo( self, luaSyntaxHighlightScripts, &rustAnalyzer).await;
                    },
                    Edits::Edit::Deletion (action) => {
                        action.Undo(self, luaSyntaxHighlightScripts, &rustAnalyzer).await;
                    },
                    Edits::Edit::RemoveLine (action) => {
                        action.Undo(self, luaSyntaxHighlightScripts, &rustAnalyzer).await;
                    },
                    Edits::Edit::NewLine (action) => {
                        action.Undo(self, luaSyntaxHighlightScripts, &rustAnalyzer).await;
                    },
                }
            }
            self.redoneBuffer.push(edits);
        }
    }

    pub async fn Redo <'a> (&mut self,
                            luaSyntaxHighlightScripts: &LuaScripts,
                            rustAnalyzer: RustAnalyzerLsp<'a>,
    ) {
        // resetting a bunch of things
        self.scrolled = std::cmp::max(
            self.mouseScrolledFlt as isize + self.scrolled as isize,
            0
        ) as usize;
        self.mouseScrolled = 0;
        self.mouseScrolledFlt = 0.0;
        self.highlighting = false;

        if let Some(edits) = self.redoneBuffer.pop() {
            for edit in &edits {
                match edit {
                    Edits::Edit::Addition (action)      => {
                        action.Redo(self, luaSyntaxHighlightScripts, rustAnalyzer).await;
                    },
                    Edits::Edit::Deletion (action)      => {
                        action.Redo(self, luaSyntaxHighlightScripts, rustAnalyzer).await;
                    },
                    Edits::Edit::RemoveLine (action) => {
                        action.Redo(self, luaSyntaxHighlightScripts, rustAnalyzer).await;
                    },
                    Edits::Edit::NewLine (action)       => {
                        action.Redo(self, luaSyntaxHighlightScripts, rustAnalyzer).await;
                    },
                }
            }
            self.changeBuffer.push(edits);
        }
    }

    pub fn Save (&mut self) {
        self.saved = true;
        let mut fileContents = String::new();
        for line in &self.lines {
            fileContents.push_str(line.as_str());
            fileContents.push('\n');
        }
        fileContents.pop();  // popping the final \n so it doesn't gradually expand over time

        std::fs::write(&self.path, fileContents).expect("Unable to write file");
    }

    pub fn MoveCursorLeftToken (&mut self) {
        self.scrolled = std::cmp::max(
            self.mouseScrolledFlt as isize + self.scrolled as isize,
            0
        ) as usize;
        self.mouseScrolled = 0;
        self.mouseScrolledFlt = 0.0;
        self.cursor.1 = std::cmp::min (
            self.cursor.1,
            self.lines[self.cursor.0].len()
        );
        
        // walking back till no longer on a space
        while self.cursor.1 > 0 && self.lines[self.cursor.0]
            .get(self.cursor.1-1..self.cursor.1)
            .unwrap_or("") == " "
        {
            self.cursor.1 -= 1;
        }
        
        let mut totalLine = String::new();
        let lineTokensRead = self.lineTokens.read();
        for token in &lineTokensRead[self.cursor.0] {
            if totalLine.len() + token.text.len() >= self.cursor.1 {
                self.cursor.1 = totalLine.len();
                return;
            }
            totalLine.push_str(token.text.as_str());
        }  // lineTokensRead is naturally dropped
    }

    pub fn FindTokenPosLeft (&mut self) -> usize {
        self.cursor.1 = std::cmp::min (
            self.cursor.1,
            self.lines[self.cursor.0].len()
        );
        let mut newCursor = self.cursor.1;

        while newCursor > 0 && self.lines[self.cursor.0]
            .get(newCursor-1..newCursor)
            .unwrap_or("") == " "
        {
            newCursor -= 1;
        }

        let mut totalLine = String::new();
        let lineTokensRead = self.lineTokens.read();
        for token in &lineTokensRead[self.cursor.0] {
            if totalLine.len() + token.text.len() >= newCursor {
                newCursor = totalLine.len();
                break;
            }
            totalLine.push_str(token.text.as_str());
        }
        drop(lineTokensRead);

        self.cursor.1 - newCursor
    }

    pub fn FindTokenPosRight (&mut self) -> usize {
        if self.lines[self.cursor.0].is_empty() {  return 0;  }

        self.cursor.1 = std::cmp::min (
            self.cursor.1,
            self.lines[self.cursor.0].len()
        );
        let mut newCursor = self.cursor.1;

        while newCursor < self.lines[self.cursor.0].len()-1 &&
            self.lines[self.cursor.0]
                .get(newCursor..newCursor + 1)
                .unwrap_or("") == " "
        {
            newCursor += 1;
        }

        let mut totalLine = String::new();
        let lineTokensRead = self.lineTokens.read();
        for token in &lineTokensRead[self.cursor.0] {
            if totalLine.len() + token.text.len() > newCursor {
                newCursor = totalLine.len() + token.text.len();
                break;
            }
            totalLine.push_str(token.text.as_str());
        }
        drop(lineTokensRead);

        newCursor - self.cursor.1
    }
    
    pub fn MoveCursorRightToken (&mut self) {
        self.scrolled = std::cmp::max(
            self.mouseScrolledFlt as isize + self.scrolled as isize,
            0
        ) as usize;
        self.mouseScrolled = 0;
        self.mouseScrolledFlt = 0.0;
        let mut totalLine = String::new();
        let lineTokensRead = self.lineTokens.read();
        for token in &lineTokensRead[self.cursor.0] {
            if token.text != " " && totalLine.len() + token.text.len() > self.cursor.1 {
                self.cursor.1 = totalLine.len() + token.text.len();
                return;
            }
            totalLine.push_str(token.text.as_str());
        }  // lineTokensRead is naturally dropped
    }
    
    pub fn MoveCursorLeft (&mut self, amount: usize, highlight: bool) {
        if !(!self.highlighting || highlight || self.cursor.0 <= self.cursorEnd.0 &&
            self.cursor.1 <= self.cursorEnd.1)
        {
            (self.cursor, self.cursorEnd) = (self.cursorEnd, self.cursor);
            self.highlighting = false;
            return;
        } else if self.highlighting && !highlight {
            self.highlighting = false;
            return;
        }
        self.scrolled = std::cmp::max(
            self.mouseScrolledFlt as isize + self.scrolled as isize,
            0
        ) as usize;
        self.mouseScrolled = 0;
        self.mouseScrolledFlt = 0.0;
        if (self.cursor.1 == 0 || self.lines[self.cursor.0].is_empty()) && self.cursor.0 > 0 {
            self.cursor.0 -= 1;
            self.cursor.1 = self.lines[self.cursor.0].len();
            return;
        }
        
        self.cursor = (
            self.cursor.0,
            std::cmp::min(
                self.cursor.1,
                self.lines[self.cursor.0].len()
            ).saturating_sub(amount)
        );
    }

    pub fn MoveCursorRight (&mut self, amount: usize, highlight: bool) {
        if self.highlighting && !highlight && self.cursor.0 < self.cursorEnd.0 ||
            self.cursor.1 < self.cursorEnd.1 && self.cursor.0 == self.cursorEnd.1
                && !highlight && self.highlighting
        {
            (self.cursor, self.cursorEnd) = (self.cursorEnd, self.cursor);
            self.highlighting = false;
            return;
        } else if self.highlighting && !highlight {
            self.highlighting = false;
            return;
        }
        self.scrolled = std::cmp::max(
            self.mouseScrolledFlt as isize + self.scrolled as isize,
            0
        ) as usize;
        self.mouseScrolled = 0;
        self.mouseScrolledFlt = 0.0;
        if self.cursor.1 >= self.lines[self.cursor.0].len() &&
            self.cursor.0 < self.lines.len() - 1
        {
            self.cursor.0 += 1;
            self.cursor.1 = 0;
            return;
        }

        self.cursor = (
            self.cursor.0,
            self.cursor.1.saturating_add(amount)
        );
    }

    pub async fn InsertChars <'a> (&mut self,
                                   chs: String,
                                   luaSyntaxHighlightScripts: &LuaScripts,
                                   rustAnalyzer: RustAnalyzerLsp<'a>,
    ) {
        self.redoneBuffer.clear();
        let mut changeBuff = vec!();

        // doesn't need to exit bc/ chars should still be added
        self.HandleHighlight(&mut changeBuff, luaSyntaxHighlightScripts, rustAnalyzer).await;

        let preCursor = self.cursor;

        self.scrolled = std::cmp::max(
            self.mouseScrolledFlt as isize + self.scrolled as isize,
            0
        ) as usize;
        self.mouseScrolled = 0;
        self.mouseScrolledFlt = 0.0;
        let length = self.lines[self.cursor.0]
            .len();
        self.lines[self.cursor.0].insert_str(
            std::cmp::min(
                self.cursor.1,
                length
            ),
            chs.as_str()
        );

        self.cursor = (
            self.cursor.0,
            std::cmp::min(
                self.cursor.1,
                length
            ) + chs.len()
        );

        changeBuff.insert(0,
            Edits::Edit::Addition(Edits::Addition {
                start: self.cursor,
                end: preCursor,
                text: chs.clone()
            })
        );
        self.changeBuffer.push(
            changeBuff
        );

        self.RecalcTokens(self.cursor.0, 0, luaSyntaxHighlightScripts).await;

        self.CreateScopeThread(self.cursor.0, self.cursor.0, rustAnalyzer);
        //(self.scopes, self.scopeJumps, self.linearScopes) = GenerateScopes(&self.lineTokens, &self.lineTokenFlags, &mut self.outlineKeywords);
    }

    pub async fn UnIndent <'a> (&mut self,
                                luaSyntaxHighlightScripts: &LuaScripts,
                                rustAnalyzer: RustAnalyzerLsp<'a>,
    ) {
        self.redoneBuffer.clear();
        self.scrolled = std::cmp::max(
            self.mouseScrolledFlt as isize + self.scrolled as isize,
            0
        ) as usize;
        self.mouseScrolled = 0;
        self.mouseScrolledFlt = 0.0;
        // checking for 4 spaces at the start
        if let Some(charSet) = &self.lines[self.cursor.0].get(..4) {
            if *charSet == "    " {
                self.changeBuffer.push(
                    vec![
                        Edits::Edit::Deletion(Edits::Deletion {
                            start: (self.cursor.0, 3),  // I think it should be 3
                            end: (self.cursor.0, 0),
                            text: "    ".to_string()
                        })
                    ]
                );

                for _ in 0..4 {  self.lines[self.cursor.0].remove(0);  }
                self.cursor.1 = self.cursor.1.saturating_sub(4);

                self.RecalcTokens(self.cursor.0, 0, luaSyntaxHighlightScripts).await;

                self.CreateScopeThread(self.cursor.0, self.cursor.0, rustAnalyzer);
                //(self.scopes, self.scopeJumps, self.linearScopes) =
                //    GenerateScopes(&self.lineTokens, &self.lineTokenFlags, &mut self.outlineKeywords);
            }
        }
    }

    pub fn CursorUp (&mut self, highlight: bool) {
        if !(!self.highlighting || highlight || self.cursor.0 <= self.cursorEnd.0 &&
            self.cursor.1 <= self.cursorEnd.1)
        {
            (self.cursor, self.cursorEnd) = (self.cursorEnd, self.cursor);
            self.highlighting = false;
        } else if self.highlighting && !highlight {
            self.highlighting = false;
        }
        self.scrolled = std::cmp::max(
            self.mouseScrolledFlt as isize + self.scrolled as isize,
            0
        ) as usize;
        self.mouseScrolled = 0;
        self.mouseScrolledFlt = 0.0;
        self.cursor = (
            self.cursor.0.saturating_sub(1),
            self.cursor.1
        );
    }

    pub fn CursorDown (&mut self, highlight: bool) {
        if self.highlighting && !highlight && self.cursor.0 < self.cursorEnd.0 ||
            self.cursor.1 < self.cursorEnd.1 && self.cursor.0 == self.cursorEnd.1
                && !highlight && self.highlighting
        {
            (self.cursor, self.cursorEnd) = (self.cursorEnd, self.cursor);
            self.highlighting = false;
        } else if self.highlighting && !highlight {
            self.highlighting = false;
        }

        self.scrolled = std::cmp::max(
            self.mouseScrolledFlt as isize + self.scrolled as isize,
            0
        ) as usize;
        self.mouseScrolled = 0;
        self.mouseScrolledFlt = 0.0;
        self.cursor = (
            std::cmp::min(
                self.cursor.0.saturating_add(1),
                self.lines.len() - 1
            ),
            self.cursor.1
        );
    }

    pub fn JumpCursor (&mut self, position: usize, scalar01: usize) {
        self.mouseScrolled = 0;
        self.mouseScrolledFlt = 0.0;
        self.cursor.0 =
            std::cmp::min(
                position,
                self.lines.len() - 1
        );
        
        // finding the starting position
        let mut startingPos = self.lines[self.cursor.0].len() * scalar01;
        for i in 0..self.lines[self.cursor.0].len() {
            startingPos += 1;
            if self.lines[self.cursor.0].get(i..i+1).unwrap_or("") != " " {
                break;
            }
        }
        self.cursor.1 = std::cmp::min(
            startingPos,
            self.lines[self.cursor.0].len()
        );
    }

    pub async fn LineBreakIn <'a> (&mut self,
                                   highlight: bool,
                                   luaSyntaxHighlightScripts: &LuaScripts,
                                   rustAnalyzer: RustAnalyzerLsp<'a>,
    ) {
        self.redoneBuffer.clear();
        self.changeBuffer.push(
            vec![
                Edits::Edit::NewLine(Edits::NewLine {
                    position: self.cursor
                })
            ]
        );
        
        self.scrolled = std::cmp::max(
            self.mouseScrolledFlt as isize + self.scrolled as isize,
            0
        ) as usize;
        self.mouseScrolled = 0;
        self.mouseScrolledFlt = 0.0;
        let length = self.lines[self.cursor.0].len();

        if length == 0 {
            self.lines.insert(self.cursor.0, "".to_string());
            let mut lineTokensWrite = self.lineTokens.write();
            lineTokensWrite[self.cursor.0].clear();
            lineTokensWrite.insert(self.cursor.0, vec!());
            drop(lineTokensWrite);  // the .write is dropped (writes can back up all the reads)
            self.lineTokenFlags.write().insert(self.cursor.0, vec!());

            self.CreateScopeThread(self.cursor.0, self.cursor.0 + 1, rustAnalyzer);
            //(self.scopes, self.scopeJumps, self.linearScopes) = GenerateScopes(&self.lineTokens, &self.lineTokenFlags, &mut self.outlineKeywords);

            self.cursor.1 = 0;
            self.CursorDown(highlight);
            return;
        }

        let rightSide = self.lines[self.cursor.0]
            .split_off(std::cmp::min(
                self.cursor.1,
            length
        ));

        self.lines.insert(
            self.cursor.0 + 1,
            rightSide,
        );
        self.lineTokens.write().insert(
            self.cursor.0 + 1,
            vec!(),
        );
        self.lineTokenFlags.write().insert(self.cursor.0 + 1, vec!());
        
        self.RecalcTokens(self.cursor.0, 0, luaSyntaxHighlightScripts).await;
        self.RecalcTokens(self.cursor.0 + 1, 0, luaSyntaxHighlightScripts).await;
        self.cursor.1 = 0;
        self.CursorDown(highlight);

        self.CreateScopeThread(self.cursor.0, self.cursor.0 + 1, rustAnalyzer);
        //(self.scopes, self.scopeJumps, self.linearScopes) = GenerateScopes(&self.lineTokens, &self.lineTokenFlags, &mut self.outlineKeywords);

    }

    pub async fn HandleHighlight <'a> (&mut self,
                                       changeBuff: &mut Vec <Edits::Edit>,
                                       luaSyntaxHighlightScripts: &LuaScripts,
                                       rustAnalyzer: RustAnalyzerLsp<'a>,
    ) -> bool {
        self.redoneBuffer.clear();
        if !self.highlighting || self.cursorEnd == self.cursor {  return false;  }
        if self.cursorEnd.0 < self.cursor.0 ||
             self.cursorEnd.0 == self.cursor.0 && self.cursorEnd.1 < self.cursor.1
        {
            if self.cursorEnd.0 == self.cursor.0 {
                changeBuff.push(Edits::Edit::Deletion(Edits::Deletion {
                    start: self.cursor,
                    end: self.cursorEnd,
                    text: self.lines[self.cursorEnd.0]
                        .get(self.cursorEnd.1..self.cursor.1)
                        .unwrap_or("")
                        .to_string()
                }));
                self.lines[self.cursorEnd.0]
                    .replace_range(self.cursorEnd.1..self.cursor.1, "");
                self.RecalcTokens(self.cursor.0, 0, luaSyntaxHighlightScripts).await;
            } else {
                let mut accumulative = String::new();
                accumulative.push_str(
                    self.lines[self.cursorEnd.0]
                        .get(self.cursorEnd.1..).unwrap_or("")
                );
                accumulative.push('\n');
                self.lines[self.cursorEnd.0]
                    .replace_range(self.cursorEnd.1.., "");
                self.RecalcTokens(self.cursorEnd.0, 0, luaSyntaxHighlightScripts).await;

                // go through any inbetween lines and delete them. Also delete one extra line so there aren't to blanks?
                let numBetween = self.cursor.0 - self.cursorEnd.0 - 1;
                for _ in 0..numBetween {
                    accumulative.push_str(
                        self.lines[self.cursorEnd.0 + 1].clone().as_str()
                    );
                    accumulative.push('\n');
                    self.lines.remove(self.cursorEnd.0 + 1);
                    self.lineTokens.write().remove(self.cursorEnd.0 + 1);
                    self.lineTokenFlags.write().remove(self.cursorEnd.0 + 1);
                }

                accumulative.push_str(
                    self.lines[self.cursorEnd.0 + 1]
                        .get(..self.cursor.1).unwrap_or("")
                );
                accumulative.push('\n');
                self.lines[self.cursorEnd.0 + 1]
                    .replace_range(..self.cursor.1, "");
                self.RecalcTokens(self.cursor.0, 0, luaSyntaxHighlightScripts).await;
                // push the next line onto the first...
                let nextLine = self.lines[self.cursorEnd.0 + 1].clone();
                accumulative.push_str(
                    nextLine.clone().as_str()
                );  // does a \n go right after this? Or is it not needed??????
                self.lines[self.cursorEnd.0].push_str(nextLine.as_str());
                self.RecalcTokens(self.cursorEnd.0, 0, luaSyntaxHighlightScripts).await;
                self.lines.remove(self.cursorEnd.0 + 1);
                self.lineTokens.write().remove(self.cursorEnd.0 + 1);
                self.lineTokenFlags.write().remove(self.cursorEnd.0 + 1);

                changeBuff.push(Edits::Edit::Deletion(Edits::Deletion {
                    start: self.cursor,
                    end: self.cursorEnd,
                    text: accumulative
                }));
                // I have no clue if this is actually correct or not; mostly the cursor position stuff
                changeBuff.push(Edits::Edit::NewLine(Edits::NewLine {
                    position: (
                        self.cursor.0 + 1,
                        0
                    )
                }));
            }

            self.highlighting = false;
            self.cursor = self.cursorEnd;
            self.CreateScopeThread(self.cursor.0, self.cursorEnd.0, rustAnalyzer);
            //(self.scopes, self.scopeJumps, self.linearScopes) =
            //    GenerateScopes(&self.lineTokens, &self.lineTokenFlags, &mut self.outlineKeywords);
            true
        } else {
            // swapping the cursor and ending points so the other calculations work
            (self.cursor, self.cursorEnd) = (self.cursorEnd, self.cursor);
            Box::pin(async move {
                self.HandleHighlight(changeBuff, luaSyntaxHighlightScripts, rustAnalyzer).await
            }).await
        }
    }

    pub fn GetSelection (&self) -> String {
        let mut occumulation = String::new();

        if !self.highlighting || self.cursor == self.cursorEnd {
            occumulation.push_str(self.lines[self.cursor.0].clone().as_str());
            occumulation.push('\n');  // fix this so that it always forces it to be pushed to a new line before
            return occumulation;
        }

        if self.cursorEnd.0 == self.cursor.0 {
            if self.cursorEnd.1 < self.cursor.1 {  // cursor on the same line
                let selection =
                    &self.lines[self.cursor.0][self.cursorEnd.1..self.cursor.1];
                occumulation.push_str(selection);
            } else {
                let selection =
                    &self.lines[self.cursor.0][self.cursor.1..self.cursorEnd.1];
                occumulation.push_str(selection);
            }
        } else if self.cursor.0 > self.cursorEnd.0 {  // cursor highlighting downwards
            let selection = &self.lines[self.cursorEnd.0][self.cursorEnd.1..];
            occumulation.push_str(selection);
            occumulation.push('\n');

            // getting the center section
            let numBetween = self.cursor.0 - self.cursorEnd.0 - 1;
            for i in 0..numBetween {
                let selection = &self.lines[self.cursorEnd.0 + 1 + i];
                occumulation.push_str(selection.clone().as_str());
                occumulation.push('\n');
            }

            let selection = &self.lines[self.cursor.0][..self.cursor.1];
            occumulation.push_str(selection);
        } else {  // cursor highlighting upwards
            let selection = &self.lines[self.cursor.0][self.cursor.1..];
            occumulation.push_str(selection);
            occumulation.push('\n');

            // getting the center section
            let numBetween = self.cursorEnd.0 - self.cursor.0 - 1;
            for i in 0..numBetween {
                let selection = &self.lines[self.cursor.0 + 1 + i];
                occumulation.push_str(selection.clone().as_str());
                occumulation.push('\n');
            }

            let selection = &self.lines[self.cursorEnd.0][..self.cursorEnd.1];
            occumulation.push_str(selection);
        }

        occumulation
    }
    
    // cursorOffset can be used to delete in multiple directions
    // if the cursorOffset is equal to numDel, it'll delete to the right
    // cursorOffset = 0 is default and dels to the left
    pub async fn DelChars <'a> (&mut self,
                                numDel: usize,
                                cursorOffset: usize,
                                luaSyntaxHighlightScripts: &LuaScripts,
                                rustAnalyzer: RustAnalyzerLsp<'a>,
    ) {
        self.redoneBuffer.clear();

        // deleting characters from scrolling
        let mut changeBuff = vec!();
        if self.HandleHighlight(&mut changeBuff, luaSyntaxHighlightScripts, rustAnalyzer).await {
            self.changeBuffer.push(changeBuff);
            return;
        }

        let preCursor = self.cursor;
        let mut deletedText = String::new();

        self.scrolled = std::cmp::max(
            self.mouseScrolledFlt as isize + self.scrolled as isize,
            0
        ) as usize;
        self.mouseScrolled = 0;
        self.mouseScrolledFlt = 0.0;
        let length = self.lines[self.cursor.0]
            .len();

        if self.cursor.1 < numDel && cursorOffset == 0 && self.lines.len() > 1 {
            // the remaining text
            deletedText.push_str(
                self.lines[self.cursor.0]
                    .get(..self.cursor.1)
                    .unwrap_or("")
            );
            let remaining = self.lines[self.cursor.0].split_off(self.cursor.1);

            self.lines.remove(self.cursor.0);
            self.lineTokens.write().remove(self.cursor.0);
            self.lineTokenFlags.write().remove(self.cursor.0);
            self.cursor.0 = self.cursor.0.saturating_sub(1);
            self.cursor.1 = self.lines[self.cursor.0].len();

            self.lines[self.cursor.0].push_str(remaining.as_str());
            self.RecalcTokens(self.cursor.0, 0, luaSyntaxHighlightScripts).await;

            changeBuff.insert(0,
                Edits::Edit::Deletion(Edits::Deletion{
                    start: self.cursor,
                    end: preCursor,
                    text: deletedText
                })
            );
            changeBuff.insert(1,
                Edits::Edit::RemoveLine(Edits::RemoveLine{
                    position: (
                        preCursor.0,
                        0
                    )
                })
            );
            self.changeBuffer.push(
                changeBuff
            );

            self.CreateScopeThread(self.cursor.0, self.cursor.0, rustAnalyzer);
            //self.linearScopes) =
            //    GenerateScopes(&self.lineTokens, &self.lineTokenFlags, &mut self.outlineKeywords);

            return;
        }
        
        self.cursor = (
            self.cursor.0,
            std::cmp::min(
                self.cursor.1,
                length
            )
        );

        let mut newCursor = self.cursor.1;
        if cursorOffset == 0 {
            newCursor = self.cursor.1.saturating_sub(numDel);
        }

        deletedText.push_str(
            self.lines[self.cursor.0]
                .get(newCursor
                    ..
                    std::cmp::min(
                        self.cursor.1.saturating_add(cursorOffset),
                        length
                )
            ).unwrap_or("")
        );
        self.lines[self.cursor.0]
            .replace_range(
                newCursor
                    ..
                    std::cmp::min(
                        self.cursor.1.saturating_add(cursorOffset),
                        length
                ),
                ""
        );

        self.cursor.1 = newCursor;

        changeBuff.insert(0,
            Edits::Edit::Deletion(Edits::Deletion{
                start: preCursor,
                end: self.cursor,
                text: deletedText
            })
        );
        self.changeBuffer.push(
            changeBuff
        );

        self.RecalcTokens(self.cursor.0, 0, luaSyntaxHighlightScripts).await;

        self.CreateScopeThread(self.cursor.0, self.cursor.0, rustAnalyzer);
        //(self.scopes, self.scopeJumps, self.linearScopes) =
        //    GenerateScopes(&self.lineTokens, &self.lineTokenFlags, &mut self.outlineKeywords);
    }

    pub async fn RecalcTokens (&mut self,
                         lineNumber: usize,
                         recursed: usize,
                         luaSyntaxHighlightScripts: &LuaScripts
    ) {
        self.saved = false;
        if lineNumber >= self.lines.len() {  return;  }

        // proper error handling actually fixed it.... who could have imagined?
        if let Some(cacheReset) = self.resetCache.get_mut(lineNumber.saturating_sub(self.lastScroll)) {
            *cacheReset = true;
        }

        let lineTokenFlagsRead = self.lineTokenFlags.read();
        let containedComment =
            lineTokenFlagsRead[lineNumber]
                .get(lineTokenFlagsRead[lineNumber].len().saturating_sub(1))
                .unwrap_or(&vec![])
                .contains(&LineTokenFlags::Comment);
        let previousEnding = lineTokenFlagsRead[lineNumber].get(
            lineTokenFlagsRead[lineNumber].len().saturating_sub(1)
        ).unwrap_or(&vec!()).clone();
        drop(lineTokenFlagsRead);
        self.lineTokens.write()[lineNumber].clear();

        let ending = self.fileName.split('.').next_back().unwrap_or("");
        let newTokens = GenerateTokens(
                    self.lines[lineNumber].clone(),
                    ending, &self.lineTokenFlags,
                    lineNumber,
                    &self.outlineKeywords,
                    luaSyntaxHighlightScripts
        ).await;
        // not being given up? crashing here
        self.lineTokens.write()[lineNumber] = newTokens;

        let lineTokenFlagsRead = self.lineTokenFlags.read();
        let currentFlags = lineTokenFlagsRead[lineNumber][
            lineTokenFlagsRead[lineNumber].len() - 1
        ].clone();
        drop(lineTokenFlagsRead);

        let empty = currentFlags.is_empty();
        if (lineNumber < self.lines.len() - 1 && !empty &&
                previousEnding != currentFlags ||
                empty && !previousEnding.is_empty()) &&
            (
                recursed < 250 && (
                    containedComment || currentFlags.contains(&LineTokenFlags::Comment)
                ) || recursed < 25
            ) {
            
            Box::pin(async move {
                self.RecalcTokens(lineNumber + 1, recursed + 1, luaSyntaxHighlightScripts).await;  // cascading any changes further down the file (kinda slow)
            }).await;
        }
        
        // recalculating variables, methods, etc...
    }

    pub fn GenerateColor (&self, token: &TokenType, text: String, colorMode: &Colors::ColorMode) -> Colored {
        match token {
            TokenType::Bracket => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                )
            },
            TokenType::SquirlyBracket => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                )
            },
            TokenType::Parentheses => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                )
            },
            TokenType::Variable => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                )
            },
            TokenType::Member => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                )
            },
            TokenType::Object => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                )
            },
            TokenType::Function => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                )
            },
            TokenType::Method => {
                color![text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                ), Italic]  // works for now ig
            },
            TokenType::Number => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                )
            },
            TokenType::Logic => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                )
            },
            TokenType::Math => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                )
            },
            TokenType::Assignment => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                )
            },
            TokenType::Endl => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                )
            },
            TokenType::Macro => {
                color![text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                ), Italic, Bold, Underline]
            },
            TokenType::Const => {
                color![text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                ), Italic]
            },
            TokenType::Barrow => {
                color![text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                ), Italic]
            },
            TokenType::Lifetime => {
                color![text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                ), Underline, Bold]
            },
            TokenType::String => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                    )
            },
            TokenType::Comment | TokenType::CommentLong => {
                if text == "todo" || text == "!" || text == "Todo" ||
                    text == "error" || text == "condition" ||
                    text == "conditions" || text == "fix" {
                    color![text.Colorize(
                            *colorMode.colorBindings
                                .syntaxHighlighting
                                .get(&(token, &colorMode.colorType))
                                .expect("Error.... no color found")
                        ), Underline]
                }  // basic but it kinda does stuff idk
                else {
                    text.Colorize(
                        *colorMode.colorBindings
                            .syntaxHighlighting
                            .get(&(token, &colorMode.colorType))
                            .expect("Error.... no color found")
                    )
                }
            },
            TokenType::Null => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                )
            },
            TokenType::Primitive => {
                text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                )
            },
            TokenType::Keyword => {
                color![text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                ), Bold, Underline]
            },
            TokenType::Unsafe => {
                color![text.Colorize(
                    *colorMode.colorBindings
                        .syntaxHighlighting
                        .get(&(token, &colorMode.colorType))
                        .expect("Error.... no color found")
                ), Italic, Underline, Bold]  // bright black is dark gray? white is gray, bright white is pure white?
            },
            TokenType::Grayed => {
                color![text, BrightWhite, Dim, Italic]
            }
        }
    }

    pub fn CheckUnderline (mut text: Vec <Colored>, (lineNumber, cursor, editing): (usize, usize, bool)) -> Vec <Colored> {
        if lineNumber != cursor || !editing {  return text;  }
        for token in text.iter_mut() {
            *token = color![token, Underline]
        } text
    }

    pub fn HighlightText (text: Colored,
                          charIndexStart: usize,
                          tokenCharCount: usize,
                          highlight: (usize, usize),
                          lineInfo: (usize, usize, bool)
    ) -> Vec <Colored> {
        // checking if the highlight range falls within the word
        if highlight.0 < charIndexStart+tokenCharCount && highlight.1 > charIndexStart {
            return
                if highlight.0 <= charIndexStart && highlight.1 >= charIndexStart + tokenCharCount {
                    CodeTab::CheckUnderline(vec![
                        color![text, OnBrightBlack],
                    ], lineInfo)  // full token
                } else if highlight.0 <= charIndexStart && highlight.1 >= charIndexStart {  // if highlight.1 was greater, the previous statement would have caught it
                    let split = text.Split(highlight.1 - charIndexStart);
                    CodeTab::CheckUnderline(vec![color![split.0, OnBrightBlack], split.1], lineInfo)  // highlight(start -> point) + point -> end
                } else if highlight.0 > charIndexStart && highlight.1 >= charIndexStart {
                    let split = text.Split(highlight.0 - charIndexStart);
                    CodeTab::CheckUnderline(vec![split.0, color![split.1, OnBrightBlack]], lineInfo)  // start -> point + highlight(point -> end)
                } else {
                    let split = text.Split(highlight.0 - charIndexStart);
                    let split2 = split.1.Split(highlight.1 - highlight.0 - charIndexStart);
                    CodeTab::CheckUnderline(vec![split.0, color![split2.0, OnBrightBlack], split2.1], lineInfo)  // start -> point1 + highlight(point1 -> point2) + point2 -> end
            };
        } CodeTab::CheckUnderline(vec![text], lineInfo)  // not a valid highlight
    }

    pub fn GetScrolledText (&mut self, area: &Rect,
                                    editingCode: bool,
                                    colorMode: &Colors::ColorMode,
                                    suggested: &str,  // the suggested auto-complete (for inline rendering)
                                    padding: u16,
    ) -> Vec <Span> {
        self.UpdateScrollingRender(area);

        // the cumulative render
        let mut tabRender = vec![];

        // getting the current line being viewed
        let scroll = std::cmp::max(
            self.scrolled as isize + self.mouseScrolled,
            0
        ) as usize;
        self.shiftCache = scroll as i32 - self.lastScroll as i32;
        self.lastScroll = scroll;

        // the maximum number of digits for the line number
        let maxLineNumberSize = self.lines.len().to_string().len() + 2;  // number of digits + 2usize;

        // iterating over every line one by one
        //    -- (maybe change this to a buffer that can be shifted as it's moved around)
        let mut i = 0;
        // the magic number of 11 is the size of the code-tab window (kinda ugly having it here but I'm too lazy to move it)
        let windowHeight = area.height as usize - 11;
        // checking the reset and scroll cache should fix any memory leaks from edge cases
        let currentMouse = (self.cursor.0, self.cursor.1, self.cursorEnd.0, self.cursorEnd.1);
        if self.resetCache.len() != windowHeight ||
           self.scrollCache.len() != windowHeight ||
           self.lastMouse != currentMouse
        {
            self.resetCache = vec![true; windowHeight];
            self.scrollCache.clear();
            self.shiftCache = 0;
            self.lastMouse = currentMouse;
        } else {
            self.UpdateCache();
        }
        for lineNumber in scroll..(scroll + windowHeight) {
            if !self.resetCache[lineNumber - scroll] {
                tabRender.push(self.scrollCache[lineNumber - scroll].clone());
                continue;
            } self.resetCache[lineNumber - scroll] = false;

            if lineNumber >= self.lines.len() {
                let mut text = " ".repeat(maxLineNumberSize - 2);
                text.push('~');
                tabRender.push(Span::FromTokens(vec![
                    color![text, Bold, Blue]
                ]));
                self.scrollCache.push(Span::FromTokens(vec![
                    color![text, Bold, Blue]
                ]));
                continue;
            }
            // getting the text for the line number
            let lineNumberText = self.GetLineNumberText(lineNumber, maxLineNumberSize);
            let colors =
                if lineNumber == self.cursor.0 {  vec![ColorType::Red, ColorType::Underline, ColorType::Bold]  }
                else {  vec![ColorType::White, ColorType::Italic]  };  // no additional coloring

            let mut lineText = vec![];
            lineText.push(lineNumberText.Colorizes(colors));

            let mut charIndex = 0;
            let width = area.width - padding - 2 - maxLineNumberSize as u16;
            self.RenderSlice(&mut charIndex, &mut lineText, lineNumber, colorMode, editingCode, width as usize, suggested);

            self.HandleScrollBar(&mut lineText, area, width, charIndex, i);

            // pushing the line
            tabRender.push(Span::FromTokens(lineText.clone()));
            self.scrollCache.push(Span::FromTokens(lineText));
            i += 1;
        } tabRender
    }

    pub fn ClearRenderCache (&mut self) {
        self.resetCache = vec![true; self.scrollCache.len()];
        self.scrollCache.clear();
        self.shiftCache = 0;
    }

    fn UpdateCache (&mut self) {
        if self.shiftCache.unsigned_abs() > self.scrollCache.len() as u32 {
            self.ClearRenderCache();
            return;
        }
        if self.shiftCache > 0 {
            self.ClearRenderCache();
            return;
            for _ in 0..self.shiftCache as usize {
                self.scrollCache.remove(0);
                self.resetCache.remove(0);
                self.resetCache.push(true);
            }
        } else if self.shiftCache < 0 {
            self.ClearRenderCache();
            return;
            for _ in 0..(-self.shiftCache) as usize {
                let size = self.scrollCache.len() - 1;
                self.scrollCache.remove(size);
                //self.resetCache.remove(size);
                self.resetCache.insert(0, true);
            }
        }
    }

    fn HandleScrollBar (&self, lineText: &mut Vec <Colored>, area: &Rect, width: u16, charIndex: usize, i: usize) {
        // the edge
        let scrollPercent = f64::min(std::cmp::max(
            self.scrolled as isize + self.mouseScrolled, 0
        ) as f64 / self.lines.len() as f64 * (area.height as f64 - 10.0),
                                     area.height as f64 - 12.0
        ) as usize;

        let pinnedPercents = self.GetPinnedPercents(area);

        let borderSpace = width as usize - charIndex;
        lineText.push(color![" ".repeat(borderSpace.saturating_sub(1))]);

        let pinned = self.IsPinned(&pinnedPercents, i);
        lineText.push(
            if pinned.0 {
                " ".Colorize(pinned.1)
            } else if i == scrollPercent || i == scrollPercent.saturating_sub(1) || i == scrollPercent + 1 {
                color![" ", OnBrightWhite]
            } else {
                color![" ", OnBrightBlack]
        });
    }

    fn IsPinned (&self, pinnedPoints: &Vec <(usize, ColorType)>, i: usize) -> (bool, ColorType) {
        for pinned in pinnedPoints {
            if pinned.0 == i {
                return (true, pinned.1);
            }
        } (false, ColorType::Default)
    }

    fn GetPinnedPercents (&self, area: &Rect) -> Vec <(usize, ColorType)> {
        let mut pinned = vec![];
        for line in &self.pinedLines {
            let scrollPercent = f64::min(std::cmp::max(
                line.0, 0
            ) as f64 / self.lines.len() as f64 * (area.height as f64 - 10.0),
                                         area.height as f64 - 12.0
            ) as usize;
            pinned.push((scrollPercent, line.1));
        } pinned
    }

    fn RenderSlice (&self,
                    charIndex: &mut usize,
                    lineText: &mut Vec <Colored>,
                    lineNumber: usize,
                    colorMode: &Colors::ColorMode,
                    editingCode: bool,
                    width: usize,
                    suggested: &str
    ) {
        let highlighted = self.CheckHighlight(lineNumber);
        let tokensRead = self.lineTokens.read();
        for token in &tokensRead[lineNumber] {
            let tokenCharCount = token.text.chars().count();

            // rendering the cursor and the split half's
            if editingCode &&
               lineNumber == self.cursor.0 &&
               self.cursor.1 >= *charIndex &&
               self.cursor.1 < *charIndex+tokenCharCount
            {
                let middle = self.cursor.1 - *charIndex;
                let left = self.GenerateColor(&token.token,
                                                       token.text[0..middle].to_string(),
                                                       colorMode
                );
                let right = self.GenerateColor(&token.token,
                                                        token.text[middle..].to_string(),
                                                        colorMode
                );

                lineText.append(&mut CodeTab::HighlightText(left,
                                                            *charIndex,
                                                            middle,
                                                            highlighted,
                                                            (lineNumber, self.cursor.0, editingCode)
                ));
                lineText.push(color!["|"]);
                lineText.append(&mut CodeTab::HighlightText(right,
                                                            *charIndex+middle,
                                                            tokenCharCount-middle,
                                                            highlighted,
                                                            (lineNumber, self.cursor.0, editingCode)
                ));
                *charIndex += 1;
            } else {
                lineText.append(&mut CodeTab::HighlightText(
                    self.GenerateColor(&token.token, token.text.clone(), colorMode),
                    *charIndex,
                    tokenCharCount,
                    highlighted,
                    (lineNumber, self.cursor.0, editingCode),
                ));
            }

            *charIndex += tokenCharCount;

            // checking the current size; making sure the text is pruned to the edge
            let overShoot = *charIndex as isize - width as isize;
            if overShoot < 0 {  continue;  }
            while *charIndex >= width {
                let token = lineText.pop().unwrap_or_default();
                let tokenSize = token.GetSize();
                *charIndex -= tokenSize;
            } break;
        }

        if self.cursor.0 == lineNumber && self.cursor.1 >= *charIndex && editingCode && *charIndex+1 < width {
            let padded = lineText[lineText.len() - 1].GetSize();
            lineText.push(color!["|"]);
            *charIndex += 1;
            if *charIndex+padded+1 < width && padded < suggested.len() {
                let suggestedText = &suggested[padded..];
                lineText.push(color![suggestedText, BrightBlack, Italic]);
                *charIndex += suggested.len() - padded;
            }
        }
    }

    // returns start, end
    pub fn CheckHighlightLines (lineNumber: usize,
                                start: (usize, usize),
                                end: (usize, usize),
    ) -> (usize, usize) {
        if lineNumber == start.0 {
            if start.0 == end.0 {  (start.1, end.1)  }
            else {  (start.1, 999)  }
        } else if lineNumber == end.0 {
            (0, end.1)
        } else if lineNumber > start.0 && lineNumber < end.0 {
            (0, 999)
        } else {
            (999, 999)
        }
    }

    // returns start, end
    fn CheckHighlight (&self, lineNumber: usize) -> (usize, usize) {
        if !self.highlighting {  return (999, 999);  }
        if self.cursor.0 == self.cursorEnd.0 {
            if self.cursor.1 < self.cursorEnd.1 {
                // left
                CodeTab::CheckHighlightLines(lineNumber, self.cursor, self.cursorEnd)
            } else {
                // right
                CodeTab::CheckHighlightLines(lineNumber, self.cursorEnd, self.cursor)
            }
        } else if self.cursor.0 < self.cursorEnd.0 {
            // the cursor is above / left of the other cursor (order is cursor, end)
            CodeTab::CheckHighlightLines(lineNumber, self.cursor, self.cursorEnd)
        } else {
            // the cursor is bellow / right of the other cursor (order is end, cursor)
            CodeTab::CheckHighlightLines(lineNumber, self.cursorEnd, self.cursor)
        }
    }

    fn GetLineNumberText (&self, lineNumber: usize, maxSize: usize) -> String {
        // choosing between the line number and lines from cursor
        let mut lineNumberText =
            if self.cursor.0 == lineNumber {  format!("{}: ", lineNumber + 1)  }  // current line number
            else {  format!("{}: ", (lineNumber as isize - self.cursor.0 as isize)
                        .unsigned_abs())  };  // distance from cursor
        lineNumberText.insert_str(0, &" ".repeat(maxSize - lineNumberText.len()));
        lineNumberText
    }

    fn UpdateScrollingRender (&mut self, area: &Rect) {
        // using the known area to adjust the scrolled position (even though this can now be done else wise..... too lazy to move it)
        let currentTime = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .expect("Time went backwards...")
            .as_millis();
        if currentTime.saturating_sub(self.pauseScroll) <= 125 {  return;  }

        if self.scrolled + SCROLL_BOUNDS >= self.cursor.0 {
            self.ScrollBranchOne(area);
        }
        if (self.scrolled + area.height as usize - 12)
            .saturating_sub(SCROLL_BOUNDS) <= self.cursor.0
        {
            self.ScrollBranchTwo(area);
        }
    }

    fn ScrollBranchOne (&mut self, area: &Rect) {
        if self.scrolled
            .saturating_sub(CENTER_BOUNDS) >=
            self.cursor.0 && !self.highlighting
        {
            let center = std::cmp::min(
                self.cursor.0
                    .saturating_sub((area.height as usize)
                        .saturating_sub(10) / 2),
                self.lines.len() - 1
            );
            self.scrolled = center;
        } else {
            self.scrolled = self.cursor.0.saturating_sub(SCROLL_BOUNDS);
            if self.highlighting {  // making sure the highlighting doesn't scroll at light speed
                // even though this is all embedded into async stuff (earlier in the call stack/depth)
                // it's fine as the async-manager uses unique threads for every async tasks.
                // The executor can deal with other futures regardless of if this current task is
                // being blocked.
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    }

    fn ScrollBranchTwo (&mut self, area: &Rect) {
        if self.scrolled + area.height as usize + CENTER_BOUNDS <=
            self.cursor.0 && !self.highlighting
        {
            let center = std::cmp::min(
                self.cursor.0
                    .saturating_sub((area.height as usize)
                        .saturating_sub(10) / 2),
                self.lines.len() - 1
            );
            self.scrolled = center;
        } else {
            self.scrolled = (self.cursor.0 + SCROLL_BOUNDS)
                .saturating_sub(area.height as usize - 12);
            if self.highlighting {  // making sure the highlighting doesn't scroll at light speed
                std::thread::sleep(std::time::Duration::from_millis(25));  // this.... probably needs to be better....
            }
        }
    }
}

impl Default for CodeTab {
    fn default() -> Self {
         CodeTab{
             cursor: (0, 0),
             lines: vec![],
             lineTokens: std::sync::Arc::new(parking_lot::RwLock::new(vec![])),
             scopeJumps: std::sync::Arc::new(parking_lot::RwLock::new(vec![])),
             scopes: std::sync::Arc::new(parking_lot::RwLock::new(ScopeNode {
                 children: vec![],
                 name: "Root".to_string(),
                 start: 0,
                 end: 0,
             })),
             linearScopes: std::sync::Arc::new(parking_lot::RwLock::new(vec![])),
             scrolled: 0,
             mouseScrolled: 0,
             mouseScrolledFlt: 0.0,
             name: "Welcome.txt"
                 .to_string(),
             fileName: "".to_string(),
             cursorEnd: (0, 0),
             highlighting: false,
             pauseScroll: 0,
             searchIndex: 0,
             searchTerm: String::new(),
             changeBuffer: vec!(),
             redoneBuffer: vec!(),
             pinedLines: vec![],
             outlineKeywords: std::sync::Arc::new(parking_lot::RwLock::new(vec!())),
             lineTokenFlags: std::sync::Arc::new(parking_lot::RwLock::new(vec!())),
             scopeGenerationHandles: vec!(),
             saved: true,
             path: String::new(),
             scrollCache: vec![],
             resetCache: vec![],
             shiftCache: 0,
             lastScroll: 0,
             lastMouse: (0, 0, 0, 0),
        }
    }
}


#[derive(Debug, Default)]
pub struct CodeTabs {
    pub tabFileNames: Vec <String>,
    pub tabs: Vec <CodeTab>,
    pub currentTab: usize,
    pub panes: Vec <usize>,  // todo! add this
}

impl CodeTabs {
    pub fn CheckScopeThreads (&mut self) {
        for tab in self.tabs.iter_mut() {
            tab.CheckScopeThreads();
        }
    }

    pub fn GetRelativeTabPosition (&self, positionX: u16, area: &Rect, paddingLeft: u16) -> u16 {
        let total = self.panes.len() as u16 + 1;
        let tabSize = (area.width - paddingLeft) / total;

        // getting the error
        let error = (area.width as f64 - paddingLeft as f64) / (total as f64) - tabSize as f64;

        let tabNumber = std::cmp::min(
            (positionX.saturating_sub(paddingLeft)) / tabSize,
            self.panes.len() as u16  // no need to sub one bc/ the main tab isn't in the vector
        );
        // error = 0.5
        // offset 0, 1, 0, 1, 0, 1
        // 0.5*(tab+1)
        let offset = (error * (tabNumber+1) as f64) as u16;
        // println!("Offset: {}", offset);
        positionX.saturating_sub(paddingLeft)
            .saturating_sub(tabSize * tabNumber)
            .saturating_sub(tabNumber)  // no clue why this is needed tbh
            .saturating_sub(offset)
    }

    /*pub fn GetTab (&mut self, area: &Rect, paddingLeft: usize, positionX: usize, lastTab: &mut usize) -> &mut CodeTab {
        let tab = self.GetTabNumber(area, paddingLeft, positionX, lastTab);
        &mut self.tabs[tab]
    }*/

    pub fn GetTabNumber (&self, area: &Rect, paddingLeft: usize, positionX: usize, lastTab: &mut usize) -> usize {
        let total = self.panes.len() + 1;
        let tabSize = (area.width as usize - paddingLeft) / total;
        let tabNumber = std::cmp::min(
            (positionX - paddingLeft) / tabSize,
            self.panes.len()  // no need to sub one bc/ the main tab isn't in the vector
        );
        if tabNumber == 0 {
            self.currentTab.clone_into(lastTab);
            self.currentTab
        } else {
            tabNumber.clone_into(lastTab);
            self.panes[tabNumber - 1]
        }
    }

    pub fn GetTabSize (&self, area: &Rect, paddingLeft: usize) -> usize {
        let total = self.panes.len() + 1;
        (area.width as usize - paddingLeft) / total
    }

    pub fn GetScrolledText (&mut self,
                            area: &Rect,
                            editingCode: bool,
                            colorMode: &Colors::ColorMode,
                            suggested: &str,
                            tabIndex: usize,
                            padding: u16,
    ) -> Vec <Span> {
        self.tabs[tabIndex].GetScrolledText(area, editingCode, colorMode, suggested, padding)
    }

    /*pub fn CloseTab (&mut self) {
        if self.tabs.len() > 1 {  // there needs to be at least one file open
            self.tabs.remove(self.currentTab);
            self.tabFileNames.remove(self.currentTab);
            self.currentTab = self.currentTab.saturating_sub(1);
        }
    }*/

    pub fn MoveTabRight (&mut self) {
        if self.currentTab < self.tabFileNames.len() - 1 {
            self.currentTab += 1;

            self.tabFileNames.swap(self.currentTab, self.currentTab - 1);
            self.tabs.swap(self.currentTab, self.currentTab - 1);
        }
    }

    pub fn MoveTabLeft (&mut self) {
        if self.currentTab > 0 {
            self.currentTab -= 1;  // there's a condition ensuring its 1 or greater

            self.tabFileNames.swap(self.currentTab, self.currentTab + 1);
            self.tabs.swap(self.currentTab, self.currentTab + 1);
        }
    }

    pub fn TabLeft (&mut self) {
        self.currentTab = self.currentTab.saturating_sub(1);
    }

    pub fn TabRight(&mut self) {
        self.currentTab = std::cmp::min(
            self.currentTab.saturating_add(1),
            self.tabFileNames.len() - 1
        );
    }

    fn GetSavedText (&self, index: usize) -> Colored {
        if self.tabs[index].saved {
            color![""]
        } else {
            color!["*", Red, Bold, Blink]
        }
    }

    pub fn GetColoredNames (&self, onTabs: bool) -> Vec <Colored> {
        let mut colored = vec!();

        if onTabs {
            for (index, tab) in self.tabFileNames.iter().enumerate() {
                let savedText = self.GetSavedText(index);
                if index == self.currentTab {
                    colored.push(
                        color![
                            format!(" ({}) ", index + 1),
                            BrightYellow,
                            Bold,
                            OnBlue,
                            Underline
                        ]
                    );
                    colored.push(
                        color![tab, White, Italic, OnBlue, Underline]
                    );
                    colored.push(color![savedText.clone(), OnBlue, Underline]);
                    colored.push(
                        color![" |", White, Bold, OnBlue, Underline]
                    );
                    continue;
                }
                colored.push(
                    color![format!(" ({}) ", index + 1), BrightYellow, Bold, Underline]
                );
                colored.push(
                    color![tab, White, Italic, Underline]
                );
                colored.push(color![savedText.clone(), Underline]);
                colored.push(
                    color![" |", White, Bold, Underline]
                );
            }
            return colored;
        }

        for (index, tab) in self.tabFileNames.iter().enumerate() {
            let savedText = self.GetSavedText(index);
            if index == self.currentTab {
                colored.push(
                    color![format!(" ({}) ", index + 1), BrightYellow, Bold, OnBrightBlack]
                );
                colored.push(
                    color![tab, White, Italic, OnBrightBlack]
                );
                colored.push(color![savedText.clone(), OnBrightBlack]);
                colored.push(
                    color![" |", White, Bold, OnBrightBlack]
                );
                continue;
            }
            colored.push(
                color![format!(" ({}) ", index + 1), BrightYellow, Bold]
            );
            colored.push(
                color![tab, White, Italic]
            );
            colored.push(savedText.clone());
            colored.push(
                color![" |", White, Bold]
            );
        }

        colored
    }
}
