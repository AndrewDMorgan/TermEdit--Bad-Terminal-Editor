// for the old syntax highlighting functions
#![allow(dead_code)]

// for some reason when I set something just to pass it
// in as a parameter, it thinks it's never read even though
// it's read in the function it's passed to
#![allow(unused_assignments)]

use mlua::{Error, FromLua, Lua, Value};
use crate::LuaScripts;

use parking_lot::{Mutex, RwLock};
use std::sync::Arc;
use crossbeam::thread;

// language file types for syntax highlighting
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub enum Languages {
    Rust,
    Cpp,
    Python,
    Null,
    Lua,
}

static LANGS: [(Languages, &str); 5] = [
    (Languages::Cpp , "cpp"),
    (Languages::Cpp , "hpp"),
    (Languages::Rust, "rs" ),
    (Languages::Python, "py"),
    (Languages::Lua, "lua"),
];


// token / syntax highlighting stuff idk
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum TokenType {
    Bracket,
    SquirlyBracket,
    Parentheses,
    Variable,
    Member,
    Object,
    Function,
    Method,
    Number,
    Logic,
    Math,
    Assignment,
    Endl,
    Macro,
    Const,
    Barrow,
    Lifetime,
    String,
    Comment,
    Null,
    Primitive,
    Keyword,
    CommentLong,
    Unsafe,
    Grayed,
}

#[derive(Debug)]
pub struct LuaTuple {
    pub token: TokenType,
    pub text: String,
}

impl FromLua for LuaTuple {
    fn from_lua(values: Value, _lua: &Lua) -> mlua::Result<Self> {
        let table: mlua::Table = match values.as_table() {
            Some(t) => t.clone(),
            _ => {
                return Err(Error::FromLuaConversionError {
                    from: "MultiValue",
                    to: "Token".to_string(),
                    message: Some("Expected a Lua table".to_string()),
                })
            }
        };

        let token;
        let tokenValue: Result <Value, _> = table.get(1);
        if tokenValue.is_ok() {
            let tokenValue = tokenValue?;
            if tokenValue.is_string() {
                token = match tokenValue.as_string_lossy().unwrap_or(String::new()).as_str() {
                    "Bracket" => Ok(TokenType::Bracket),
                    "SquirlyBracket" => Ok(TokenType::SquirlyBracket),
                    "Parentheses" => Ok(TokenType::Parentheses),
                    "Variable" => Ok(TokenType::Variable),
                    "Member" => Ok(TokenType::Member),
                    "Object" => Ok(TokenType::Object),
                    "Function" => Ok(TokenType::Function),
                    "Method" => Ok(TokenType::Method),
                    "Number" => Ok(TokenType::Number),
                    "Logic" => Ok(TokenType::Logic),
                    "Math" => Ok(TokenType::Math),
                    "Assignment" => Ok(TokenType::Assignment),
                    "Endl" => Ok(TokenType::Endl),
                    "Macro" => Ok(TokenType::Macro),
                    "Const" => Ok(TokenType::Const),
                    "Barrow" => Ok(TokenType::Barrow),
                    "Lifetime" => Ok(TokenType::Lifetime),
                    "String" => Ok(TokenType::String),
                    "Comment" => Ok(TokenType::Comment),
                    "Primitive" => Ok(TokenType::Primitive),
                    "Keyword" => Ok(TokenType::Keyword),
                    "CommentLong" => Ok(TokenType::CommentLong),
                    "Unsafe" => Ok(TokenType::Unsafe),
                    "Grayed" => Ok(TokenType::Grayed),
                    _ => Ok(TokenType::Null),
                };
            } else {
                //let text = format!("Invalid token arg {:?}", tokenValue);
                //panic!("{}", text);
                //token = Err(Error::UserDataTypeMismatch);
                token = Ok(TokenType::Null)
            }
        } else {
            token = Err(Error::UserDataTypeMismatch);
        }

        let text;
        let textValue: Result <Value, _> = table.get(2);
        if textValue.is_ok() {
            let textValue = textValue?;
            if textValue.is_string() {
                text = Ok(textValue.as_string_lossy().unwrap_or(String::new()));
            } else {
                text = Err(Error::UserDataTypeMismatch);
            }
        } else {
            text = Err(Error::UserDataTypeMismatch);
        }

        Ok( LuaTuple {token: token?, text: text?} )
    }
}


// tracking both the next line flags, but also individual token flags for variable/outline generation (auto complete stuff ig)
#[derive(Debug, Clone, PartialEq)]
pub enum LineTokenFlags {
    Comment,
    Parameter,
    Generic,
    List,
    String,
    // Expression,  // leaving this out for now (not sure if brackets matter here)
}

#[derive(Debug, Clone)]
pub enum OutlineType {
    Variable,
    Struct,
    Enum,
    Variant,  // the variant of enum
    Function,
    Member,
    Generic,
    Lifetime,
    Mod,
}

#[derive(Debug, Clone)]
pub struct OutlineKeyword {
    pub keyword: String,
    pub kwType: OutlineType,
    pub typedType: Option <String>,  // only for explicitly annotated types -- basic error tracking?
    pub resultType: Option <String>,  // for function outputs
    pub childKeywords: Vec <OutlineKeyword>,
    pub scope: Vec <usize>,  // for tracking private/public methods and members
    pub public: Option <bool>,  // true == public; false == private (or None)
    pub mutable: bool,  // false == no; true == yes

    // name, type; the type has to be explicitly annotated for this to be picked up
    pub parameters: Option <Vec <(String, Option <String>)>>,
    pub lineNumber: usize,
    pub implLines: Vec <usize>,
}

impl OutlineKeyword {
    pub fn EditScopes (outline: &mut Vec <OutlineKeyword>, scope: &Vec <usize>, lineNumber: usize) {
        //let mut outlineWrite = outline.write();
        for keyword in outline.iter_mut() {
            if keyword.lineNumber == lineNumber {
                keyword.scope = scope.clone();
                if matches!(keyword.kwType, OutlineType::Function | OutlineType::Enum | OutlineType::Struct) {
                    keyword.scope.pop();  // seems to fix things? idk
                }
                // outlineWrite is dropped
                //drop(outlineWrite);
                return;
            }
        }
        // outlineWrite is dropped
        //drop(outlineWrite);
    }

    pub fn GetValidScoped (outline: &Arc <parking_lot::RwLock <Vec <OutlineKeyword>>>, scope: &Vec <usize>) -> Vec <OutlineKeyword> {
        let mut valid: Vec <OutlineKeyword> = vec!();
        let outlineRead = outline.read();
        for keyword in outlineRead.iter() {
            if keyword.scope.is_empty() || scope.as_slice().starts_with(&keyword.scope.as_slice()) {
                valid.push(keyword.clone());
            }
        }
        // outlineRead is dropped
        drop(outlineRead);
        valid
    }

    pub fn TryFindKeyword (outline: &Arc <parking_lot::RwLock <Vec <OutlineKeyword>>>, queryWord: String) -> Option <OutlineKeyword> {
        let outlineRead = outline.read();
        for keyword in outlineRead.iter() {
            if queryWord == keyword.keyword {
                return Some(keyword.clone());
            }
        }
        // outlineRead is dropped
        drop(outlineRead);
        None
    }
}


#[derive(Clone)]
pub enum TokenFlags {
    Comment,  // has priority of everything including strings which overrule everything else
    String,  // has priority over everything (including chars)
    //Char,  // has 2nd priority (overrules generics)
    Generic,
    Null,
}

lazy_static::lazy_static! {
    static ref LINE_BREAKS: [String; 26] = [
        " ".to_string(),
        "@".to_string(),
        "?".to_string(),
        "=".to_string(),
        "|".to_string(),
        "#".to_string(),
        ".".to_string(),
        ",".to_string(),
        "(".to_string(),
        ")".to_string(),
        "[".to_string(),
        "]".to_string(),
        "{".to_string(),
        "}".to_string(),
        ";".to_string(),
        ":".to_string(),
        "!".to_string(),
        "/".to_string(),
        "+".to_string(),
        "-".to_string(),
        "*".to_string(),
        "&".to_string(),
        "'".to_string(),
        "\"".to_string(),
        "<".to_string(),
        ">".to_string(),
    ];
}

fn GenerateTokenStrs (text: &str) -> Arc <Mutex <Vec <String>>> {
    let mut current = "".to_string();
    let tokenStrs: Arc <Mutex <Vec <String>>> = Arc::new(Mutex::new(vec![]));
    let tokenStrsClone = Arc::clone(&tokenStrs);
    let mut tokenStrsClone = tokenStrsClone.lock();
    for character in text.chars() {
        if !current.is_empty() && LINE_BREAKS.contains(&character.to_string()) {
            tokenStrsClone.push(current.clone());
            current.clear();
            current.push(character);
            tokenStrsClone.push(current.clone());
            current.clear();
            continue
        }

        current.push(character);
        let mut valid = false;
        for breaker in LINE_BREAKS.iter() {
            if !current.contains(breaker) {  continue;  }
            valid = true;
            break;
        }

        if !valid {  continue;  }
        tokenStrsClone.push(current.clone());
        current.clear();
    }
    tokenStrsClone.push(current);
    tokenStrs
}

fn GetLineFlags (tokenStrs: &Vec <String>) -> Vec <TokenFlags> {
    let mut flags: Vec <TokenFlags> = vec!();
    let mut currentFlag = TokenFlags::Null;  // the current flag being tracked
    for (index, token) in tokenStrs.iter().enumerate() {
        let emptyString = &"".to_string();
        let nextToken = tokenStrs.get(index + 1).unwrap_or(emptyString).as_str();
        let prevToken = tokenStrs.get(index.saturating_sub(1)).unwrap_or(emptyString);

        let newFlag =
            match token.as_str() {
                "/" if nextToken == "/" => TokenFlags::Comment,
                //"*" if nextToken == "/" => TokenFlags::Null,
                "\"" if !matches!(currentFlag, TokenFlags::Comment) && prevToken != "\\" => {  //  | TokenFlags::Char
                    if matches!(currentFlag, TokenFlags::String) {  TokenFlags::Null}
                    else {  TokenFlags::String}
                },
                "<" if !matches!(currentFlag, TokenFlags::Comment | TokenFlags::String) => TokenFlags::Generic,  //  | TokenFlags::Char
                //">" if matches!(currentFlag, TokenFlags::Generic) => TokenFlags::Null,  (unfortunately the lifetimes are needed... not sure how to fix it properly without tracking more info)
                /*"'" if !matches!(currentFlag, TokenFlags::Comment | TokenFlags::String | TokenFlags::Generic) => {
                    if matches!(currentFlag, TokenFlags::Char) {  TokenFlags::Null}
                    else {  TokenFlags::Char  }
                },*/
                _ => currentFlag,
            };

        currentFlag = newFlag;
        flags.push(currentFlag.clone());
    } flags
}

fn GetPythonToken (
    strToken: &str,
    flags: &Vec <TokenFlags>,
    index: usize,
    (nextToken, prevToken): (&str, &String)
) -> TokenType {
    match strToken {
        _s if matches!(flags[index], TokenFlags::Comment) => TokenType::Comment,
        _s if matches!(flags[index], TokenFlags::String) => TokenType::String,  //  | TokenFlags::Char
        "if" | "for" | "while" | "in" | "else" |
        "break" | "elif" | "return" | "continue" |
        "import" | "and" | "not" | "or" | "@" |
        "try" | "except" => TokenType::Keyword,
        "def" => TokenType::Function,
        " " => TokenType::Null,
        "int" | "float" | "string" | "bool" | "list" |
        "range" | "round" | "min" | "max" | "abs" => TokenType::Primitive,
        "[" | "]" => TokenType::Bracket,
        "(" | ")" => TokenType::Parentheses,
        ":" => TokenType::SquirlyBracket,
        s if s.chars().next().map_or(false, |c| {
            c.is_ascii_digit()
        }) => TokenType::Number,
        "=" | "-" if nextToken == ">" => TokenType::Keyword,
        ">" if prevToken == "=" => TokenType::Keyword,
        ">" if prevToken == "-" => TokenType::Keyword,
        "=" if prevToken == ">" || prevToken == "<" || prevToken == "=" => TokenType::Logic,
        s if (prevToken == "&" && s == "&") || (prevToken == "|" && s == "|") => TokenType::Logic,
        s if (nextToken == "&" && s == "&") || (nextToken == "|" && s == "|") => TokenType::Logic,
        ">" | "<" | "False" | "True" | "!" => TokenType::Logic,
        "=" if nextToken == "=" => TokenType::Logic,
        "=" if prevToken == "+" || prevToken == "-" || prevToken == "*" || prevToken == "/" => TokenType::Math,
        "=" if nextToken == "+" || nextToken == "-" || nextToken == "*" || nextToken == "/" => TokenType::Math,
        "+" | "-" | "*" | "/" => TokenType::Math,
        "=" => TokenType::Assignment,
        "\"" | "'" => TokenType::String,
        "class" | "self" | "super" => TokenType::Object,
        _s if strToken.to_uppercase() == *strToken => TokenType::Const,
        _s if prevToken == "." && prevToken[..1].to_uppercase() == prevToken[..1] => TokenType::Method,
        _s if prevToken == "." => TokenType::Member,
        _s if strToken[..1].to_uppercase() == strToken[..1] => TokenType::Function,
        _ => TokenType::Null,
    }
}

fn GetCppToken (
    strToken: &str,
    flags: &Vec <TokenFlags>,
    index: usize,
    (nextToken, prevToken): (&str, &String)
) -> TokenType {
    match strToken {
        _s if matches!(flags[index], TokenFlags::Comment) => TokenType::Comment,
        _s if matches!(flags[index], TokenFlags::String) => TokenType::String,  //  | TokenFlags::Char
        "if" | "for" | "while" | "in" | "else" |
        "break" | "loop" | "goto" | "return" | "std" |
        "const" | "static" | "template" | "continue" |
        "include" | "#" | "alloc" | "malloc" |
        "using" | "namespace" => TokenType::Keyword,
        " " => TokenType::Null,
        "int" | "float" | "double" | "string" | "char" | "short" |
        "long" | "bool" | "unsigned" => TokenType::Primitive,
        "[" | "]" => TokenType::Bracket,
        "(" | ")" => TokenType::Parentheses,
        ":" => TokenType::Member,
        s if s.chars().next().map_or(false, |c| {
            c.is_ascii_digit()
        }) => TokenType::Number,
        "=" | "-" if nextToken == ">" => TokenType::Keyword,
        ">" if prevToken == "=" => TokenType::Keyword,
        ">" if prevToken == "-" => TokenType::Keyword,
        "=" if prevToken == ">" || prevToken == "<" || prevToken == "=" => TokenType::Logic,
        s if (prevToken == "&" && s == "&") || (prevToken == "|" && s == "|") => TokenType::Logic,
        s if (nextToken == "&" && s == "&") || (nextToken == "|" && s == "|") => TokenType::Logic,
        ">" | "<" | "false" | "true" | "!" => TokenType::Logic,
        "=" if nextToken == "=" => TokenType::Logic,
        "=" if prevToken == "+" || prevToken == "-" || prevToken == "*" || prevToken == "/" => TokenType::Math,
        "=" if nextToken == "+" || nextToken == "-" || nextToken == "*" || nextToken == "/" => TokenType::Math,
        "+" | "-" | "*" | "/" => TokenType::Math,
        "{" | "}" | "|" => TokenType::SquirlyBracket,
        "=" => TokenType::Assignment,
        ";" => TokenType::Endl,
        "&" => TokenType::Barrow,
        "\"" | "'" => TokenType::String,
        "public" | "private" | "this" | "class" | "struct" => TokenType::Object,
        _s if strToken.to_uppercase() == *strToken => TokenType::Const,
        _s if prevToken == "." && prevToken[..1].to_uppercase() == prevToken[..1] => TokenType::Method,
        _s if prevToken == "." => TokenType::Member,
        _s if strToken[..1].to_uppercase() == strToken[..1] => TokenType::Function,
        _ => TokenType::Null,
    }
}

fn GetRustToken (
    strToken: &str,
    flags: &Vec <TokenFlags>,
    index: usize,
    tokenStrs: &Vec <String>,
    (nextToken, prevToken, lastToken): (&str, &String, &TokenType)
) -> TokenType {
    match strToken {  // rust
        _s if matches!(flags[index], TokenFlags::Comment) => TokenType::Comment,
        _s if matches!(flags[index], TokenFlags::String) => TokenType::String,  //  | TokenFlags::Char
        "!" if matches!(lastToken, TokenType::Macro) => TokenType::Macro,
        "unsafe" | "get_unchecked" => TokenType::Unsafe,  // add all unsafe patterns
        "if" | "for" | "while" | "in" | "else" |
        "break" | "loop" | "match" | "return" | "std" |
        "const" | "static" | "dyn" | "type" | "continue" |
        "use" | "mod" | "None" | "Some" | "Ok" | "Err" |
        "async" | "await" | "default" | "derive" |
        "as" | "?" | "ref" => TokenType::Keyword,
        " " => TokenType::Null,
        "i32" | "isize" | "i16" | "i8" | "i128" | "i64" |
        "u32" | "usize" | "u16" | "u8" | "u128" | "u64" |
        "f16" | "f32" | "f64" | "f128" | "String" |
        "str" | "Vec" | "bool" | "char" | "Result" |
        "Option" | "Debug" | "Clone" | "Copy" | "Default" |
        "new" => TokenType::Primitive,
        "[" | "]" => TokenType::Bracket,
        "(" | ")" => TokenType::Parentheses,
        "#" => TokenType::Macro,
        _s if nextToken == "!" => TokenType::Macro,
        ":" => TokenType::Member,
        s if s.chars()
                    .next()
                    .unwrap_or(' ')
                    .is_ascii_digit()
        => TokenType::Number,
        "=" | "-" if nextToken == ">" => TokenType::Keyword,
        ">" if prevToken == "=" || prevToken == "-" => TokenType::Keyword,
        "=" if prevToken == ">" || prevToken == "<" || prevToken == "=" => TokenType::Logic,
        s if (prevToken == "&" && s == "&") || (prevToken == "|" && s == "|") => TokenType::Logic,
        s if (nextToken == "&" && s == "&") || (nextToken == "|" && s == "|") => TokenType::Logic,
        ">" | "<" | "false" | "true" | "!" => TokenType::Logic,
        "=" if nextToken == "=" => TokenType::Logic,
        "=" if prevToken == "+" || prevToken == "-" || prevToken == "*" || prevToken == "/" => TokenType::Math,
        "=" if nextToken == "+" || nextToken == "-" || nextToken == "*" || nextToken == "/" => TokenType::Math,
        "+" | "-" | "*" | "/" => TokenType::Math,
        "{" | "}" | "|" => TokenType::SquirlyBracket,
        "let" | "=" | "mut" => TokenType::Assignment,
        ";" => TokenType::Endl,
        _ => GetComplexRustTokens(
            strToken,
            index,
            tokenStrs,
            (
                nextToken,
                prevToken
            )
        ),
    }
}

fn GetComplexRustTokens (
    strToken: &str,
    index: usize,
    tokenStrs: &Vec <String>,
    (nextToken, prevToken): (&str, &String)
) -> TokenType {
    match strToken {
        "&" => TokenType::Barrow,
        "'" if index + 2 < tokenStrs.len() &&
            tokenStrs[index + 2] != "'" &&
            tokenStrs[index.saturating_sub(2)] != "'"
            => TokenType::Lifetime,
        _ if prevToken == "'" && nextToken != "'" => TokenType::Lifetime,
        "\"" | "'" => TokenType::String,
        _ if prevToken == "'" => TokenType::String,
        "enum" | "pub" | "struct" | "impl" | "self" | "Self" => TokenType::Object,
        _s if strToken.to_uppercase() == *strToken => TokenType::Const,
        _s if (prevToken == "." || prevToken == ":") &&
            strToken[..1].to_uppercase() == strToken[..1]
            => TokenType::Method,
        _s if prevToken == "." || prevToken == ":" => TokenType::Member,
        "fn" => TokenType::Function,
        _s if strToken[..1].to_uppercase() == strToken[..1] => TokenType::Function,
        _ => TokenType::Null,
    }
}

fn GetTokens (
    tokenStrs: &Vec <String>,
    flags: Vec <TokenFlags>,
    fileType: &str
) -> Vec <(TokenType, String)> {
    // track strings and track chars and generic inputs (this might allow for detecting lifetimes properly)
    let mut tokens: Vec <(TokenType, String)> = vec!();
    for (index, strToken) in tokenStrs.iter().enumerate() {
        if strToken.is_empty() && index > 0 { continue;  }
        let emptyString = &"".to_string();
        let nextToken = tokenStrs.get(index + 1).unwrap_or(emptyString).as_str();
        let prevToken = {
            if index > 0 {
                &tokenStrs[index - 1]
            } else {  &"".to_string()  }
        };

        let mut language = Languages::Null;  // the default
        for (lang, extension) in LANGS.iter() {
            if *extension == fileType {
                language = lang.clone();
                break;
            }
        }

        let nullCase = &(TokenType::Null, "".to_string());
        let lastToken = &tokens.last().unwrap_or(
            nullCase
        ).0;
        tokens.push((
            match language {
                Languages::Python => GetPythonToken(
                    strToken,
                    &flags,
                    index,
                    (nextToken, prevToken)
                ),
                Languages::Cpp => GetCppToken(
                    strToken,
                    &flags,
                    index,
                    (nextToken, prevToken)
                ),
                Languages::Rust => GetRustToken(
                    strToken,
                    &flags,
                    index,
                    &tokenStrs,
                    (
                        nextToken,
                        prevToken,
                        lastToken
                    )
                ),
                _ => {TokenType::Null}
            },
            strToken.clone()));
    } tokens
}

fn GenerateLineTokenFlags (
    lineTokenFlags: &Arc <RwLock <Vec <Vec <Vec <LineTokenFlags>>>>>,
    tokens: &Vec <LuaTuple>,
    previousFlagSet: &Vec <LineTokenFlags>,
    text: &String,
    lineNumber: usize,
) {
    // generating the new set
    let mut currentFlags = previousFlagSet.clone();
    for (index, token) in tokens.iter().enumerate() {
        let lastTokenText = tokens[index.saturating_sub(1)].text.clone();

        // not a great system for generics, but hopefully it'll work for now
        match token.text.as_str() {
            // removing comments can still happen within // comments
            "/" if lastTokenText == "*" && currentFlags.contains(&LineTokenFlags::Comment) => {
                RemoveLineFlag(&mut currentFlags, LineTokenFlags::Comment);
            }

            _ if matches!(token.token, TokenType::Comment) => {},
            // generics
            "<" if text.contains("fn") ||
                text.contains("struct") ||
                text.contains("impl") => { currentFlags.push(LineTokenFlags::Generic); }
            ">" if currentFlags.contains(&LineTokenFlags::Generic) => {
                RemoveLineFlag(&mut currentFlags, LineTokenFlags::Generic);
            }

            // strings (and char-strings ig)
            "\"" | "'" => {
                if currentFlags.contains(&LineTokenFlags::String) {
                    RemoveLineFlag(&mut currentFlags, LineTokenFlags::String);
                } else { currentFlags.push(LineTokenFlags::String); }
            }

            // Parameters
            "(" => { currentFlags.push(LineTokenFlags::Parameter); }
            ")" if currentFlags.contains(&LineTokenFlags::Parameter) => {
                RemoveLineFlag(&mut currentFlags, LineTokenFlags::Parameter);
            }

            // List
            "[" => { currentFlags.push(LineTokenFlags::List); }
            "]" if currentFlags.contains(&LineTokenFlags::List) => {
                RemoveLineFlag(&mut currentFlags, LineTokenFlags::List);
            }

            // Comments (only /* && */  not //)
            "*" if lastTokenText == "/" => { currentFlags.push(LineTokenFlags::Comment); }

            _ => {}
        }

        // the .write is being called in a loop, but writes block all other threads and
        // this is async so it's better to stall this one than the main one
        let mut tokenFlagsWrite = lineTokenFlags.write();
        tokenFlagsWrite[lineNumber].push(currentFlags.clone());
        drop(tokenFlagsWrite);  // dropped the write
    }
}

fn HandleKeyword (
    tokens: &Vec <LuaTuple>,
    lineTokenFlags: &Arc <RwLock <Vec <Vec <Vec <LineTokenFlags>>>>>,
    mut currentContainer: &mut Option <OutlineKeyword>,
    text: &String,
    tokenText: &String,
    lineNumber: usize,
    nonSet: &mut bool,
    prevTokens: &Vec <&TokenType>,
    index: usize,
) {
    let mut passedEq = false;
    match tokenText.as_str() {
        "=" => {  passedEq = true;  },
        "struct" | "enum" | "fn" | "mod" => {
            let keyword = OutlineKeyword {
                keyword: String::new(),
                kwType: match tokenText.as_str() {
                    "mod" => OutlineType::Mod,
                    "struct" => OutlineType::Struct,
                    "fn" => OutlineType::Function,
                    _ => OutlineType::Enum,
                },
                typedType: None,
                resultType: None,
                childKeywords: vec!(),
                scope: vec!(),
                public: Some(text.contains("pub")),
                mutable: false,
                parameters: None,
                lineNumber,
                implLines: vec!(),
            };

            currentContainer.replace(keyword);
        }
        "let" if currentContainer.is_none() => {
            let keyword = OutlineKeyword {
                keyword: String::new(),
                kwType: OutlineType::Variable,
                typedType: None,
                resultType: None,
                childKeywords: vec!(),  // figure this out :(    no clue how to track the children
                scope: vec!(),
                public: None,
                mutable: false,
                parameters: None,
                lineNumber,
                implLines: vec!(),
            };

            currentContainer.replace(keyword);
        },
        "mut" if currentContainer.is_some() => {
            match &mut currentContainer {
                Some(keyword) => {
                    keyword.mutable = true;
                },
                _ => {}
            }
        },
        // the conditions seem to screen fine when doing an if or while let (and other random things)
        txt if !matches!(txt, " " | "(" | "Some" | "Ok" | "mut") &&
            currentContainer.is_some()
            => HandleBinding(
                tokens,
                lineTokenFlags,
                currentContainer,
                txt,
                tokenText,
                lineNumber,
                nonSet,
                prevTokens,
                index,
                passedEq),
        _ => {}
    }
}

fn HandleBinding (
    tokens: &Vec <LuaTuple>,
    lineTokenFlags: &Arc <RwLock <Vec <Vec <Vec <LineTokenFlags>>>>>,
    mut currentContainer: &mut Option <OutlineKeyword>,
    txt: &str,
    tokenText: &String,
    lineNumber: usize,
    nonSet: &mut bool,
    prevTokens: &Vec <&TokenType>,
    index: usize,
    passedEq: bool
) {
    {
        if txt.get(0..1).unwrap_or("") == "_"  {
            *currentContainer = None;
            *nonSet = true;
        }

        if let Some(keyword) = &mut currentContainer {
            if keyword.keyword.is_empty() {
                keyword.keyword = tokenText.to_string();
            } else if prevTokens.len() > 1 &&
                tokens[index.saturating_sub(2)].text == ":" &&
                !passedEq && keyword.typedType == None
            {
                if matches!(keyword.kwType, OutlineType::Function) {
                    HandleFunctionDef(
                        keyword,
                        tokens,
                        lineTokenFlags,
                        tokenText,
                        index,
                        lineNumber
                    );
                } else {
                    keyword.typedType = Some(tokenText.to_string());
                }
            }
        }
    }
}

fn HandleFunctionDef (
    keyword: &mut OutlineKeyword,
    tokens: &Vec <LuaTuple>,
    lineTokenFlags: &Arc <RwLock <Vec <Vec <Vec <LineTokenFlags>>>>>,
    tokenText: &String,
    index: usize,
    lineNumber: usize
) {
    if keyword.parameters.is_none() {  keyword.parameters = Some(vec!());  }
    if let Some(parameters) = &mut keyword.parameters {
        parameters.push((
            tokens[index.saturating_sub(3)].text.clone(),
            Some({  // collecting all terms until the parameter field ends or a ','
                let mut text = tokenText.clone();
                let tokenFlagsRead = lineTokenFlags.read();
                for nextIndex in index+1..tokens.len() {
                    if tokens[nextIndex].text == "," ||
                        !tokenFlagsRead[lineNumber][nextIndex]
                            .contains(&LineTokenFlags::Parameter)
                    {  break;  }
                    text.push_str(&tokens[nextIndex].text.clone());
                }
                // tokenFlagsRead is dropped
                drop(tokenFlagsRead);
                text
            })
        ));
    }
}

fn HandleDefinitions(
    tokens: &Vec <LuaTuple>,
    lineTokenFlags: &Arc <RwLock <Vec <Vec <Vec <LineTokenFlags>>>>>,
    outline: &Arc <RwLock <Vec <OutlineKeyword>>>,
    text: &String,
    lineNumber: usize
) {
    let mut nonSet = false;
    let mut prevTokens: Vec <&TokenType> = vec!();
    let mut currentContainer: Option <OutlineKeyword> = None;
    for (index, token) in tokens.iter().enumerate() {
        // dealing with the outline portion now... (does this need to be in another for-loop?)
        if !nonSet {
            HandleKeyword(
                tokens,
                lineTokenFlags,
                &mut currentContainer,
                text,
                &token.text,
                lineNumber,
                &mut nonSet,
                &prevTokens,
                index
            );
        }

        prevTokens.push(&token.token);
    }
    let mut newOutline: Vec <OutlineKeyword> = vec!();
    let outlineRead = outline.read();
    for keyWord in outlineRead.iter() {
        if keyWord.lineNumber == lineNumber {  continue;  }
        newOutline.push(keyWord.clone());
    }
    drop(outlineRead);  // dropped the read

    let mut outlineWrite = outline.write();
    outlineWrite.clear();  // clearing so this is fine
    for keyWord in newOutline {
        outlineWrite.push(keyWord);
    }
    drop(outlineWrite);  // dropped the .write

    HandleImpl(currentContainer, outline, tokens, text, lineNumber);
}

fn HandleImpl (
    currentContainer: Option <OutlineKeyword>,
    outline: &Arc <RwLock <Vec <OutlineKeyword>>>,
    tokens: &Vec <LuaTuple>,
    text: &String,
    lineNumber: usize
) {
    if let Some(container) = currentContainer {
        // writing to outline; drops in the same line as called
        // should be fine sense this is called right after being cleared
        outline.write().push(container);
    } else if text.contains("impl") {
        let mut queryName = None;
        for index in (0..tokens.len()).rev() {
            if !matches!(tokens[index].token, TokenType::Function) {  continue;  }
            queryName = Some(tokens[index].text.clone());
        }
        HandleGettingImpl(outline, &queryName, lineNumber);
    }
}

fn HandleGettingImpl (
    outline: &Arc <RwLock <Vec <OutlineKeyword>>>,
    queryName: &Option <String>,
    lineNumber: usize
) {
    if let Some(queryName) = queryName {
        // was just cleared so should be fine
        let mut outlineWrite = outline.write();
        for container in outlineWrite.iter_mut() {
            if container.keyword != *queryName {  continue;  }
            container.implLines.push(lineNumber);
            break;
        }
        // outlineWrite is dropped here
        drop(outlineWrite);
    }
}

pub async fn GenerateTokens (
    text: String, fileType: &str,
    lineTokenFlags: &Arc <RwLock <Vec <Vec <Vec <LineTokenFlags>>>>>,
    lineNumber: usize,
    outline: &Arc <RwLock <Vec <OutlineKeyword>>>,
    luaSyntaxHighlightScripts: &LuaScripts,
) -> Vec <LuaTuple> {
    let tokenStrs = GenerateTokenStrs(text.as_str());

    // start a lua execution thread here
    let mut language = Languages::Null;  // the default
    for (lang, extension) in LANGS.iter() {
        if *extension == fileType {
            language = lang.clone();
            break;
        }
    }

    // calling the lua script (dealing with annoying async stuff)
    let script: Arc <&mlua::Function>;
    let highlightingScript = luaSyntaxHighlightScripts.lock().unwrap();
    if highlightingScript.contains_key(&language) {
        script = Arc::clone(&Arc::from(&highlightingScript[&language]));
    } else {
        script = Arc::clone(&Arc::from(&highlightingScript[&Languages::Null]));
    }

    // spawning the threads
    let mut tokens: Vec <LuaTuple> = vec!();
    let mut line: Vec <Vec <LineTokenFlags>> = vec!();  // all of these have to be pre-allocated (and initialized)
    let mut previousFlagSet: &Vec <LineTokenFlags> = &vec!();
    let _ = thread::scope(|s| {
        let tokensWrapped: Arc<Mutex<Vec <LuaTuple>>> = Arc::new(Mutex::new(vec!()));

        let scriptClone = Arc::clone(&script);
        let tokensClone = Arc::clone(&tokensWrapped);
        let tokenStrsClone = Arc::clone(&tokenStrs);
        let handle = s.spawn(move |_| {
            let result = scriptClone.call(tokenStrsClone.lock().clone());
            *tokensClone.lock() = result.unwrap();
        });


        // intermediary code
        let mut tokenFlagsWrite = lineTokenFlags.write();
        while tokenFlagsWrite.len() < lineNumber + 1 {
            tokenFlagsWrite.push(vec!());  // new line
        }
        drop(tokenFlagsWrite);  // dropped the .write

        // dealing with the line token stuff (running while lua is doing async stuff)
        let tokenFlagsRead = lineTokenFlags.read();
        if lineNumber != 0 && tokenFlagsRead[lineNumber - 1].len() != 0 {
            previousFlagSet = {
                line = tokenFlagsRead[lineNumber - 1].clone();
                line.last().unwrap()
            };
        }
        drop(tokenFlagsRead);  // dropped the read

        let mut tokenFlagsWrite = lineTokenFlags.write();
        tokenFlagsWrite[lineNumber].clear();
        drop(tokenFlagsWrite);  // dropped the write


        // joining the thread
        let _ = handle.join();
        tokens = Arc::try_unwrap(tokensWrapped)
            .unwrap()
            .into_inner();
    }).unwrap();

    // handling everything that requires the tokens to be calculated
    GenerateLineTokenFlags(lineTokenFlags,
                           &tokens,
                           &previousFlagSet,
                           &text,
                           lineNumber
    );

    let tokenFlagsRead = lineTokenFlags.read();
    for i in 0..tokens.len() {
        if tokenFlagsRead[lineNumber][i].contains(&LineTokenFlags::Comment) {
            tokens[i].token = TokenType::CommentLong;
        }
    }
    drop(tokenFlagsRead);  // dropped the read

    HandleDefinitions(&tokens,
                      lineTokenFlags,
                      outline,
                      &text, lineNumber
    );
    
    tokens
}

fn RemoveLineFlag(currentFlags: &mut Vec<LineTokenFlags>, removeFlag: LineTokenFlags) {
    for (i, flag) in currentFlags.iter().enumerate() {
        if *flag == removeFlag {
            currentFlags.remove(i);
            break;
        }
    }
}

// application stuff
#[derive(Debug, Clone)]
pub struct ScopeNode {
    pub children: Vec <ScopeNode>,
    pub name: String,
    pub start: usize,
    pub end: usize,
}

impl ScopeNode {
    pub fn GetNode (&self, scope: &mut Vec <usize>) -> &ScopeNode {
        let index = scope.pop();

        if index.is_none() {
            return self;
        }

        self.children[index.unwrap()].GetNode(scope)
    }

    pub fn Push (&mut self, scope: &mut Vec <usize>, name: String, start: usize) -> usize {
        let index = scope.pop();
        if index.is_none() {
            self.children.push(
                ScopeNode {
                    children: vec![],
                    name,
                    start,
                    end: 0
                }
            );
            return self.children.len() - 1;
        }
        
        self.children[index.unwrap()].Push(scope, name, start)
    }

    pub fn SetEnd (&mut self, scope: &mut Vec <usize>, end: usize) {
        let index = scope.pop();
        if index.is_none() {
            self.end = end;
            return;
        }

        self.children[index.unwrap()].SetEnd(scope, end);
    }
}

fn GetImplKeywordIndex (outline: &mut Vec <OutlineKeyword>) -> Vec <usize> {
    // make this a non-clone at some point; me lazy rn (always lazy actually...)    I actually did it!
    let mut implKeywordIndex: Vec <usize> = vec![];

    //let mut outlineWrite = outline.write();
    for (index, keyword) in outline.iter_mut().enumerate() {
        // handling impls and mods
        match keyword.kwType {
            OutlineType::Enum | OutlineType::Function |
            OutlineType::Struct | OutlineType::Mod => {
                // checking each successive scope
                if !keyword.scope.is_empty() {  // 1 for the impl and 1 for the method? idk
                    implKeywordIndex.push(index);
                }
            }
            _ => {}
        }
    }
    // outlineWrite is dropped here
    //drop(outlineWrite);
    implKeywordIndex
}

fn HandleKeywordIndexes (
    outline: &mut Vec <OutlineKeyword>,
    implKeywordIndex: Vec <usize>,
    scopeJumps: &Vec <Vec <usize>>,
    root: &ScopeNode
) {
    for keywordIndex in implKeywordIndex {
        //let outlineRead = outline.read();

        // the first scope should be the impl or mod, right?
        let keyword = outline[keywordIndex].clone();
        let mut newScope: Option <Vec <usize>> = None;

        //if keywordIndex >= outline.len() || outline[keywordIndex].scope.is_empty() {  return;  }
        //if outline[keywordIndex].scope[0] >= root.children.len() {  return;  }
        if root.children.len() < outline[keywordIndex].scope[0] {  continue;  }
        let scopeStart = root.children[outline[keywordIndex].scope[0]].start;

        //drop(outlineRead);
        //let mut outlineWrite = outline;//.write();

        for otherKeyword in outline.iter_mut() {
            if otherKeyword.implLines.contains(&scopeStart) {
                otherKeyword.childKeywords.push(keyword);
                newScope = Some(scopeJumps[otherKeyword.lineNumber].clone());
                break;
            }
        }

        if let Some(newScope) = newScope {
            outline[keywordIndex].scope = newScope;
        }

        // makings ure there aren't any active writes
        //drop(outlineWrite);
    }
}

fn HandleKeywords (
    tokenLines: &Arc <RwLock <Vec <Vec <LuaTuple>>>>,
    lineFlags: &Arc <RwLock <Vec <Vec <Vec <LineTokenFlags>>>>>,
    outline: &mut Vec <OutlineKeyword>,
    scopeJumps: &Vec <Vec <usize>>,
    root: &ScopeNode
) -> Vec <OutlineKeyword> {
    // getting a write channel for outline
    // this will block any read calls until completion
    // anything inbetween this and the dropping of its memory
    // should be optimized and performant to not stall
    // the main thread (which calls lots of reads)
    //let outlineRead = outline.read();
    //let outlineSize = outline.len();
    // write is used on this so reading has to be very sparing and controlled
    //drop(outlineRead);

    // !!!the memory leak has to due with this vector
    let mut newKeywords: Vec <OutlineKeyword> = Vec::new();
    for keyword in outline.iter_mut() {
        //{
        //let outlineRead = outline.read();
        //let keywordOption = &outline.get(keywordIndex);
        //if keywordOption.is_none() {
            // error; probably an out of data thread (there should be future ones to take over)
            //drop(outlineRead);
        //    return vec!();
        //}
        //let keyword = keywordOption.unwrap();

        if !matches!(keyword.kwType, OutlineType::Enum | OutlineType::Struct)
            { continue; }
        //}  // outline read is dropped; same with keyword

        //let mut outlineWrite = outline.write();
        if !keyword.childKeywords.is_empty() {
            keyword.childKeywords.clear()
        }
        // freeing the .write to allow reading
        //drop(outlineWrite);

        // getting the following members
        //let outlineRead = outline.read();
        if keyword.lineNumber > scopeJumps.len() {  continue;  }
        //drop(outlineRead);  // dropping the read

        // there are no active reads or writes here (in terms of local)
        // the memory leak is inside here
        HandleKeywordsLoop(
            tokenLines,
            lineFlags,
            scopeJumps,
            // tracking the index and outline to allow
            // more controlled .read's and .writes
            keyword,
            &mut newKeywords,
            root
        );
    } newKeywords
}

fn HandleKeywordsLoop (
    tokenLines: &Arc <RwLock <Vec <Vec <LuaTuple>>>>,
    lineFlags: &Arc <RwLock <Vec <Vec <Vec <LineTokenFlags>>>>>,
    scopeJumps: &Vec <Vec <usize>>,
    keyword: &mut OutlineKeyword,
    //(outline, keywordIndex): (&mut Vec <OutlineKeyword>, usize),
    newKeywords: &mut Vec<OutlineKeyword>,
    root: &ScopeNode,
) {
    //let outlineRead = outline.read();
    //drop(outlineRead);  // dropping the read
    // no local reads

    'lines: for lineNumber in
        keyword.lineNumber+1..
            ({
                let mut scope = scopeJumps[keyword.lineNumber].clone();
                scope.reverse();
                root.GetNode(&mut scope).end
            })
    {
        //let tokenLinesRead = tokenLines.read();

        let mut public = false;
        let mut currentContainer: Option <OutlineKeyword> = None;
        // this is the issue right here
        let tokensSize = tokenLines.read()[lineNumber].len();
        for index in 0..tokensSize {
        //for (index, token) in tokenLines.read()[lineNumber].iter().enumerate() {
            //let tokenText: String;
            {
                //if lineNumber > tokenLines.read().len() || index > tokenLines.read()[lineNumber].len() {  return;  }
                // index error
                let tokenLinesRead = tokenLines.read();
                //if lineNumber >= tokenLinesRead.len() || index >= tokenLinesRead[lineNumber].len() {  return;  }
                if tokenLinesRead[lineNumber][index].text == "}" { break 'lines; }
                else if tokenLinesRead[lineNumber][index].text == "pub" { public = true; }

                //tokenText = tokenLinesRead[lineNumber][index].text.clone();
                drop(tokenLinesRead);
            }
            if matches!(tokenLines.read()[lineNumber][index].token, TokenType::Comment | TokenType::String)
                {  continue;  }

            // no active read's or writes here (safe to continue)
            // wrong, the iter takes a reference.... (fixed now)
            // the memory leak is inside here
            HandleScopeChecks(
                tokenLines,
                lineFlags,
                &mut currentContainer,
                scopeJumps,
                keyword,
                index,
                index,
                lineNumber,
                public
            );
        }

        // dropping the read channel/guard (bruh, clearly there was a read.....) me dumb
        //drop(tokenLinesRead);

        if let Some(container) = currentContainer {
            // !!!!This next line is the continuation of the leak
            newKeywords.push(container.clone());

            //let mut outlineWrite = outline.write();
            keyword.childKeywords.push(container);  // adding the variant
            //drop(outlineWrite);  // dropping the .write
        }
    }
}

fn HandleScopeChecks (
    tokenLines: &Arc <RwLock <Vec <Vec <LuaTuple>>>>,
    lineFlags: &Arc <RwLock <Vec <Vec <Vec <LineTokenFlags>>>>>,
    currentContainer: &mut Option <OutlineKeyword>,
    scopeJumps: &Vec <Vec <usize>>,
    keyword: &mut OutlineKeyword,
    textIndex: usize,
    index: usize,
    lineNumber: usize,
    public: bool
) {
    // getting than dropping a read channel
    //let lineFlagsRead = lineFlags.read();
    let condition = lineFlags.read()[lineNumber][index].contains(&LineTokenFlags::Parameter);
    //drop(lineFlagsRead);  // quickly dropped

    if condition && currentContainer.is_some() {
        // no active local read's or writes
        // !the memory leak isn't in here. It's somewhere else in this function
        HandleKeywordParameterSet(
            tokenLines,
            lineFlags,
            currentContainer,
            keyword,
            textIndex,
            index,
            lineNumber
        );
    } else if !matches!(tokenLines.read()[lineNumber][textIndex].text.as_str(), " " | "(" | "Some" | "Ok" | "_" | "mut" | "," | "pub") && currentContainer.is_none() {
        //let outlineRead = outline.read();
        let newKey = OutlineKeyword {
            keyword: tokenLines.read()[lineNumber][textIndex].text.clone(),
            kwType: {
                if matches!(keyword.kwType, OutlineType::Enum) {
                    OutlineType::Variant
                } else {
                    OutlineType::Member
                }
            },
            typedType: None,
            resultType: None,
            childKeywords: vec!(),
            scope: scopeJumps[keyword.lineNumber].clone(),
            public: Some(public),  // it inherits the parents publicity
            mutable: false,
            parameters: None,
            lineNumber,
            implLines: vec!(),
        };
        //drop(outlineRead);  // dropped (no functions calls are between creation & death)
        // !!!!!this line is the root of the memory leak
        currentContainer.replace(newKey);
    }
}

fn HandleKeywordParameterSet (
    tokenLines: &Arc <RwLock <Vec <Vec <LuaTuple>>>>,
    lineFlags: &Arc <RwLock <Vec <Vec <Vec <LineTokenFlags>>>>>,
    mut currentContainer: &mut Option <OutlineKeyword>,
    keyword: &mut OutlineKeyword,
    textIndex: usize,
    index: usize,
    lineNumber: usize,
) {
    let mut parameterType = String::new();
    if tokenLines.read()[lineNumber][textIndex].text == "(" {
        // no local active reads or writes
        HandleKeywordParameter(
            tokenLines,
            lineFlags,
            currentContainer,
            keyword,
            parameterType,
            index,
            lineNumber
        );
    } else if tokenLines.read()[lineNumber][textIndex].text == ":" {
        // opening reads for lineFlags and tokenLines
        let lineFlagsRead = lineFlags.read();
        let tokenLinesRead = tokenLines.read();

        // !!! There are active read's so don't call functions that need to read or write from these

        for newCharIndex in index + 2..tokenLinesRead[lineNumber].len() {
            if lineFlagsRead[lineNumber][index].contains(&LineTokenFlags::Comment) ||
                tokenLinesRead[lineNumber][textIndex].text == "," {  break;  }
            let string = &tokenLinesRead[lineNumber][newCharIndex].text.clone();
            parameterType.push_str(string);
        }
        // dropping the read for lineFlags
        drop(lineFlagsRead);

        if let Some(container) = &mut currentContainer {
            container.parameters = Some(
                vec![(
                    tokenLinesRead[lineNumber][index.saturating_sub(1)].text.clone(),
                    Some(parameterType)
                )]
            );
        }
        // the read for tokenLines drops here
        drop(tokenLinesRead);
    }
}

fn HandleKeywordParameter (
    tokenLines: &Arc <RwLock <Vec <Vec <LuaTuple>>>>,
    lineFlags: &Arc <RwLock <Vec <Vec <Vec <LineTokenFlags>>>>>,
    mut currentContainer: &mut Option <OutlineKeyword>,
    keyword: &mut OutlineKeyword,
    mut parameterType: String,
    index: usize,
    lineNumber: usize,
) {
    let tokenLinesRead = tokenLines.read();
    let lineFlagsRead = lineFlags.read();

    // getting the parameters type
    for newCharIndex in index + 1..tokenLinesRead[lineNumber].len() {
        if !lineFlagsRead[lineNumber][index].contains(&LineTokenFlags::Parameter) {
            break;
        }
        parameterType.push_str(&tokenLinesRead[lineNumber][newCharIndex].text.clone());
    }
    drop(tokenLinesRead);
    drop(lineFlagsRead);

    // active write for outline here
    //let mut outlineWrite = outline.write();

    if keyword.parameters.is_none() {  keyword.parameters = Some(Vec::new());  }

    if let Some(container) = &mut currentContainer {
        let params = vec![(container.keyword.clone(), Some(parameterType))];
        if let Some(parameters) = &mut keyword.parameters {
            for param in params {
                parameters.push(param);
            }
        }
        //container.parameters = Some(params);  // I don't think this line is needed?
    }
    // making sure no writes are active
    //drop(outlineWrite);
}

pub fn UpdateKeywordOutline (
    tokenLines: &Arc <RwLock <Vec <Vec <LuaTuple>>>>,
    lineFlags: &Arc <RwLock <Vec <Vec <Vec <LineTokenFlags>>>>>,
    outline: &mut Vec <OutlineKeyword>,
    scopeJumps: &Vec <Vec <usize>>,
    root: &ScopeNode
) {
    let implKeywordIndex = GetImplKeywordIndex(outline);
    HandleKeywordIndexes(outline, implKeywordIndex, scopeJumps, root);

    // it's in here... (the mem leak)
    let mut newKeywords = HandleKeywords(tokenLines, lineFlags, outline, scopeJumps, root);

    //let mut outlineWrite = outline.write();
    while let Some(newKeyword) = newKeywords.pop() {
        outline.push(newKeyword);
    }
    // making sure there are no active writes
    //drop(outlineWrite);
}

static VALID_NAMES_NEXT: [&str; 5] = [
    "fn",
    "struct",
    "enum",
    "impl",
    "mod",
];

static VALID_NAMES_TAKE: [&str; 6] = [
    "for",
    "while",
    "if",
    "else",
    "match",
    "loop",
];

pub fn GenerateScopes (
    tokenLines: &Arc <RwLock <Vec <Vec <LuaTuple>>>>,
    _lineFlags: &Arc <RwLock <Vec <Vec <Vec <LineTokenFlags>>>>>,
    outlineOriginal: &Arc <RwLock <Vec <OutlineKeyword>>>,
) -> (ScopeNode, Vec <Vec <usize>>, Vec <Vec <usize>>) {
    // creating a clone of outline to avoid external influence that may lead to a memory leak
    // beforehand multiple threads were appending to the outline at the same time
    // duplicating various elements
    let mut outline = &mut outlineOriginal.read().clone();
    // confirmed, the memory leak is inside here

    // opening a reading channel for tokenLines (writing is very rare, so I'm not worried)
    // this pauses until any write channels are closed (there can be multiple read channels)
    let tokenLinesRead = tokenLines.read();

    // tracking the scope (functions = new scope; struct/enums = new scope; for/while = new scope)
    let mut rootNode = ScopeNode {
        children: vec![],
        name: "Root".to_string(),
        start: 0,
        end: tokenLinesRead.len().saturating_sub(1),
    };

    let mut jumps: Vec <Vec <usize>> = vec!();
    let mut linearized: Vec <Vec <usize>> = vec!();

    let mut currentScope: Vec <usize> = vec!();
    for (lineNumber, tokens) in tokenLinesRead.iter().enumerate() {
        HandleBracketsLayer(
            &mut outline,
            &mut rootNode,
            &mut jumps,
            &mut linearized,
            &mut currentScope,
            tokens,
            lineNumber
        );
    }

    // dropping the memory for reading tokenLines
    // hopefully anything needing to write has a brief chance here
    // !!!!!!! Make sure there aren't any writes to tokenLines or it'll crash
    drop(tokenLinesRead);

    // updating the original
    let mut outlineWrite = outlineOriginal.write();
    outlineWrite.clear();
    while let Some(keyword) = outline.pop() {
        outlineWrite.push(keyword);
    } drop(outlineWrite);

    // replace all the logic to fit in here, probably. It would make it simpler?
    // remove any references within the basic syntax highlighting method?
    // this isn't doing a whole-lot, but it's logic will be necessary at some point
    // updating the keywords outline
    // the memory leak is in this function
    //UpdateKeywordOutline(tokenLines, lineFlags, outline, &jumps, &rootNode);

    // updating the original
    /*let mut outlineWrite = outlineOriginal.write();
    outlineWrite.clear();
    while let Some(keyword) = outline.pop() {
        outlineWrite.push(keyword);
    } drop(outlineWrite);*/

    (rootNode, jumps, linearized)
}

fn HandleBracketsLayer (
    outline: &mut Vec <OutlineKeyword>,
    rootNode: &mut ScopeNode,
    jumps: &mut Vec <Vec <usize>>,
    linearized: &mut Vec <Vec <usize>>,
    currentScope: &mut Vec <usize>,
    tokens: &Vec <LuaTuple>,
    lineNumber: usize
) {
    let mut bracketDepth = 0isize;
    // track the depth; if odd than based on the type of bracket add scope; if even do nothing (scope opened and closed on the same line)
    // use the same on functions to determine if the scope needs to continue or end on that line
    for (index, token) in tokens.iter().enumerate() {
        CalculateBracketDepth(
            tokens,
            &token.token,
            &mut bracketDepth,
            &token.text,
            index
        );

        // checking for something to define the name
        if matches!(token.token, TokenType::Keyword | TokenType::Object | TokenType::Function) {
            if !(VALID_NAMES_NEXT.contains(&token.text.trim()) || VALID_NAMES_TAKE.contains(&token.text.trim())) {
                continue;
            }

            let invalid = CheckForScopeName(
                tokens,
                rootNode,
                currentScope,
                linearized,
                &token.text,
                &mut bracketDepth,
                index,
                lineNumber
            );
            if invalid {  break;  }
        }
    }

    UpdateBracketDepth(outline, rootNode, currentScope, linearized, jumps, &mut bracketDepth, lineNumber);
}

fn CalculateBracketDepth (
    tokens: &Vec <LuaTuple>,
    token: &TokenType,
    bracketDepth: &mut isize,
    name: &String,
    index: usize,
) {
    let lastToken = {
        if index == 0 {  &LuaTuple { token: TokenType::Null, text: String::new() }  }
        else {  &tokens[index - 1]  }
    };
    if !(matches!(token, TokenType::Comment) || (lastToken.text == "\"" || lastToken.text == "'") && matches!(token, TokenType::String)) {
        // checking bracket depth
        if name == "{" {
            *bracketDepth += 1;
        } else if name == "}" {
            *bracketDepth -= 1;
        }
    }
}

fn CheckForScopeName (
      tokens: &Vec <LuaTuple>,
      rootNode: &mut ScopeNode,
      currentScope: &mut Vec <usize>,
      linearized: &mut Vec <Vec <usize>>,
      name: &String,
      bracketDepth: &mut isize,
      index: usize,
      lineNumber: usize
) -> bool {
    // checking the scope to see if ti continues or ends on the same line
    let mut brackDepth = 0isize;
    for (indx, token) in tokens.iter().enumerate() {
        let lastToken = {
            if indx == 0 {  &LuaTuple { token: TokenType::Null, text: String::new() }  }
            else {  &tokens[indx - 1]  }
        };
        if !(matches!(token.token, TokenType::Comment) || lastToken.text == "\"" && matches!(token.token, TokenType::String)) &&
            indx > index
        {
            if name == "{" {
                brackDepth += 1;
            } else if name == "}" {
                brackDepth -= 1;
            }
        }
    }

    UpdateScopes(
        tokens,
        rootNode,
        currentScope,
        linearized,
        name,
        bracketDepth,
        &mut brackDepth,
        index,
        lineNumber
    )
}

fn UpdateScopes (
    tokens: &Vec <LuaTuple>,
    rootNode: &mut ScopeNode,
    currentScope: &mut Vec <usize>,
    linearized: &mut Vec <Vec <usize>>,
    name: &String,
    bracketDepth: &mut isize,
    brackDepth: &mut isize,
    index: usize,
    lineNumber: usize
) -> bool {
    // check bracket-depth here so any overlapping marks are accounted for
    if *bracketDepth < 0 && *brackDepth > 0 {  // not pushing any jumps bc/ it'll be done later
        let mut scopeCopy = currentScope.clone();
        scopeCopy.reverse();
        rootNode.SetEnd(&mut scopeCopy, lineNumber);
        currentScope.pop();
    }

    // adding the new scope if necessary
    if *brackDepth > 0 {
        if VALID_NAMES_NEXT.contains(&name.trim()) {
            let nextName = tokens
                .get(index + 2)
                .unwrap_or( &LuaTuple { token: TokenType::Null, text: String::new() } )
                .text
                .clone();

            let mut scopeCopy = currentScope.clone();
            scopeCopy.reverse();
            let mut goodName = name.clone();
            goodName.push(' ');
            goodName.push_str(nextName.as_str());
            let newScope = rootNode.Push(&mut scopeCopy, goodName, lineNumber);
            currentScope.push(newScope);
            linearized.push(currentScope.clone());

            *bracketDepth = 0;
            return true;
        } else if VALID_NAMES_TAKE.contains(&name.trim()) {
            let mut scopeCopy = currentScope.clone();
            scopeCopy.reverse();
            let goodName = name.clone();
            let newScope = rootNode.Push(&mut scopeCopy, goodName, lineNumber);
            currentScope.push(newScope);
            linearized.push(currentScope.clone());

            *bracketDepth = 0;
            return true;
        }
    } false
}

fn UpdateBracketDepth (
    outline: &mut Vec <OutlineKeyword>,
    rootNode: &mut ScopeNode,
    currentScope: &mut Vec <usize>,
    linearized: &mut Vec <Vec <usize>>,
    jumps: &mut Vec <Vec <usize>>,
    bracketDepth: &mut isize,
    lineNumber: usize
) {
    // updating the scope based on the brackets
    if *bracketDepth > 0 {
        let mut scopeCopy = currentScope.clone();
        scopeCopy.reverse();
        let newScope = rootNode.Push(&mut scopeCopy, "{ ... }".to_string(), lineNumber);
        currentScope.push(newScope);
        jumps.push(currentScope.clone());
        linearized.push(currentScope.clone());
        OutlineKeyword::EditScopes(outline, &currentScope, lineNumber);
    } else if *bracketDepth < 0 {
        jumps.push(currentScope.clone());
        OutlineKeyword::EditScopes(outline, &currentScope, lineNumber);
        let mut scopeCopy = currentScope.clone();
        scopeCopy.reverse();
        rootNode.SetEnd(&mut scopeCopy, lineNumber);
        currentScope.pop();
    } else {
        jumps.push(currentScope.clone());
        OutlineKeyword::EditScopes(outline, &currentScope, lineNumber);
    }
}


