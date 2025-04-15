// snake case is just bad
#![allow(non_snake_case)]

// for some reason when I set something just to pass it
// in as a parameter, it thinks it's never read even though
// it's read in the function
#![allow(unused_assignments)]

// language file types for syntax highlighting
#[derive(Clone)]
pub enum Languages {
    Rust,
    Cpp,
    Python,
    Null,
}

static LANGS: [(Languages, &str); 4] = [
    (Languages::Cpp , "cpp"),
    (Languages::Cpp , "hpp"),
    (Languages::Rust, "rs" ),
    (Languages::Python, "py"),
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
    Lifetime,  // goo luck figuring this out vs. a regular string.........
    String,
    Comment,
    Null,
    Primitive,
    Keyword,
    CommentLong,
    Unsafe,
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
        for keyword in outline {
            if keyword.lineNumber == lineNumber {
                keyword.scope = scope.clone();
                if matches!(keyword.kwType, OutlineType::Function | OutlineType::Enum | OutlineType::Struct) {
                    keyword.scope.pop();  // seems to fix things? idk
                }
                return;
            }
        }
    }

    pub fn GetValidScoped (outline: &Vec <OutlineKeyword>, scope: &Vec <usize>) -> Vec <OutlineKeyword> {
        let mut valid: Vec <OutlineKeyword> = vec!();
        for keyword in outline {
            if keyword.scope.is_empty() || scope.as_slice().starts_with(&keyword.scope.as_slice()) {
                valid.push(keyword.clone());
            }
        } valid
    }

    pub fn TryFindKeyword (outline: &Vec <OutlineKeyword>, queryWord: String) -> Option <OutlineKeyword> {
        for keyword in outline {
            if queryWord == keyword.keyword {
                return Some(keyword.clone());
            }
        }
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

fn GenerateTokenStrs (text: &str) -> Vec <String> {
    let mut current = "".to_string();
    let mut tokenStrs: Vec <String> = vec!();
    for character in text.chars() {
        if !current.is_empty() && LINE_BREAKS.contains(&character.to_string()) {
            tokenStrs.push(current.clone());
            current.clear();
            current.push(character);
            tokenStrs.push(current.clone());
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
        tokenStrs.push(current.clone());
        current.clear();
    }
    tokenStrs.push(current);
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
    flags: &Vec <TokenFlags>,
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
                    flags,
                    index,
                    (nextToken, prevToken)
                ),
                Languages::Cpp => GetCppToken(
                    strToken,
                    flags,
                    index,
                    (nextToken, prevToken)
                ),
                Languages::Rust => GetRustToken(
                    strToken,
                    flags,
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
    lineTokenFlags: &mut Vec <Vec <Vec <LineTokenFlags>>>,
    tokens: &Vec <(TokenType, String)>,
    previousFlagSet: &Vec <LineTokenFlags>,
    text: &String,
    lineNumber: usize,
) {
    // generating the new set
    let mut currentFlags = previousFlagSet.clone();
    for (index, (token, tokenText)) in tokens.iter().enumerate() {
        let lastTokenText = tokens[index.saturating_sub(1)].1.clone();

        // not a great system for generics, but hopefully it'll work for now
        match tokenText.as_str() {
            // removing comments can still happen within // comments
            "/" if lastTokenText == "*" && currentFlags.contains(&LineTokenFlags::Comment) => {
                RemoveLineFlag(&mut currentFlags, LineTokenFlags::Comment);
            }

            _ if matches!(token, TokenType::Comment) => {},
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
        lineTokenFlags[lineNumber].push(currentFlags.clone());
    }
}

fn HandleKeyword (
    tokens: &Vec <(TokenType, String)>,
    lineTokenFlags: &Vec <Vec <Vec <LineTokenFlags>>>,
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
    tokens: &Vec <(TokenType, String)>,
    lineTokenFlags: &Vec <Vec <Vec <LineTokenFlags>>>,
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
                tokens[index.saturating_sub(2)].1 == ":" &&
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
    tokens: &Vec <(TokenType, String)>,
    lineTokenFlags: &Vec <Vec <Vec <LineTokenFlags>>>,
    tokenText: &String,
    index: usize,
    lineNumber: usize
) {
    if keyword.parameters.is_none() {  keyword.parameters = Some(vec!());  }
    if let Some(parameters) = &mut keyword.parameters {
        parameters.push((
            tokens[index.saturating_sub(3)].1.clone(),
            Some({  // collecting all terms until the parameter field ends or a ','
                let mut text = tokenText.clone();
                for nextIndex in index+1..tokens.len() {
                    if tokens[nextIndex].1 == "," ||
                        !lineTokenFlags[lineNumber][nextIndex]
                            .contains(&LineTokenFlags::Parameter)
                    {  break;  }
                    text.push_str(&tokens[nextIndex].1.clone());
                }
                text
            })
        ));
    }
}

fn HandleDefinitions(
    tokens: &Vec <(TokenType, String)>,
    lineTokenFlags: &Vec <Vec <Vec <LineTokenFlags>>>,
    outline: &mut Vec <OutlineKeyword>,
    text: &String,
    lineNumber: usize
) {
    let mut nonSet = false;
    let mut prevTokens: Vec <&TokenType> = vec!();
    let mut currentContainer: Option <OutlineKeyword> = None;
    for (index, (token, tokenText)) in tokens.iter().enumerate() {
        // dealing with the outline portion now... (does this need to be in another for-loop?)
        if !nonSet {
            HandleKeyword(
                tokens,
                lineTokenFlags,
                &mut currentContainer,
                text,
                tokenText,
                lineNumber,
                &mut nonSet,
                &prevTokens,
                index
            );
        }

        prevTokens.push(token);
    }
    let mut newOutline: Vec <OutlineKeyword> = vec!();
    for keyWord in outline.iter() {
        if keyWord.lineNumber == lineNumber {  continue;  }
        newOutline.push(keyWord.clone());
    }

    outline.clear();
    for keyWord in newOutline {
        outline.push(keyWord);
    }

    HandleImpl(currentContainer, outline, tokens, text, lineNumber);
}

fn HandleImpl (
    currentContainer: Option <OutlineKeyword>,
    outline: &mut Vec <OutlineKeyword>,
    tokens: &Vec <(TokenType, String)>,
    text: &String,
    lineNumber: usize
) {
    if let Some(container) = currentContainer {
        outline.push(container);
    } else if text.contains("impl") {
        let mut queryName = None;
        for index in (0..tokens.len()).rev() {
            if !matches!(tokens[index].0, TokenType::Function) {  continue;  }
            queryName = Some(tokens[index].1.clone());
        }
        HandleGettingImpl(outline, &queryName, lineNumber);
    }
}

fn HandleGettingImpl (
    outline: &mut Vec <OutlineKeyword>,
    queryName: &Option <String>,
    lineNumber: usize
) {
    if let Some(queryName) = queryName {
        for container in outline.iter_mut() {
            if container.keyword != *queryName {  continue;  }
            container.implLines.push(lineNumber);
            break;
        }
    }
}

pub fn GenerateTokens (
    text: String, fileType: &str,
    mut lineTokenFlags: &mut Vec <Vec <Vec <LineTokenFlags>>>,
    lineNumber: usize,
    mut outline: &mut Vec<OutlineKeyword>,
) -> Vec <(TokenType, String)> {
    let tokenStrs = GenerateTokenStrs(text.as_str());

    // getting any necessary flags
    let flags = GetLineFlags (&tokenStrs);
    
    let mut tokens = GetTokens(&tokenStrs, &flags, fileType);

    // dealing with the line token stuff
    while lineTokenFlags.len() < lineNumber + 1 {
        lineTokenFlags.push(vec!());  // new line
    }

    let line: Vec <Vec <LineTokenFlags>>;
    let previousFlagSet: &Vec <LineTokenFlags> = {
        if lineNumber == 0 || lineTokenFlags[lineNumber - 1].len() == 0 {  &vec!()  }
        else {
            line = lineTokenFlags[lineNumber - 1].clone();
            line.last().unwrap()
        }
    };

    lineTokenFlags[lineNumber].clear();
    
    GenerateLineTokenFlags(&mut lineTokenFlags,
                           &tokens,
                           &previousFlagSet,
                           &text,
                           lineNumber
    );

    HandleDefinitions(&tokens,
                      &lineTokenFlags,
                      &mut outline,
                      &text, lineNumber
    );

    for i in 0..tokens.len() {
        if lineTokenFlags[lineNumber][i].contains(&LineTokenFlags::Comment) {
            tokens[i].0 = TokenType::CommentLong;
        }
    }
    
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
#[derive(Debug)]
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
    } implKeywordIndex
}

fn HandleKeywordIndexes (
    outline: &mut Vec <OutlineKeyword>,
    implKeywordIndex: Vec <usize>,
    scopeJumps: &Vec <Vec <usize>>,
    root: &ScopeNode
) {
    for keywordIndex in implKeywordIndex {
        // the first scope should be the impl or mod, right?
        let keyword = outline[keywordIndex].clone();
        let mut newScope: Option <Vec <usize>> = None;
        if root.children.len() < outline[keywordIndex].scope[0] {  continue;  }
        let scopeStart = root.children[outline[keywordIndex].scope[0]].start;
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
    }
}

fn HandleKeywords (
    tokenLines: &[Vec <(TokenType, String)>],
    lineFlags: &Vec <Vec <Vec <LineTokenFlags>>>,
    outline: &mut Vec <OutlineKeyword>,
    scopeJumps: &Vec <Vec <usize>>,
    root: &ScopeNode
) -> Vec <OutlineKeyword> {
    let mut newKeywords: Vec <OutlineKeyword> = Vec::new();
    for keyword in outline.iter_mut() {
        if !matches!(keyword.kwType, OutlineType::Enum | OutlineType::Struct)
            {  continue;  }
        if !keyword.childKeywords.is_empty() {
            keyword.childKeywords.clear()
        }
        // getting the following members
        if keyword.lineNumber > scopeJumps.len() {  continue;  }

        HandleKeywordsLoop(
            tokenLines,
            lineFlags,
            scopeJumps,
            keyword,
            &mut newKeywords,
            root
        );

    } newKeywords
}

fn HandleKeywordsLoop (
    tokenLines: &[Vec <(TokenType, String)>],
    lineFlags: &Vec <Vec <Vec <LineTokenFlags>>>,
    scopeJumps: &Vec <Vec <usize>>,
    keyword: &mut OutlineKeyword,
    newKeywords: &mut Vec<OutlineKeyword>,
    root: &ScopeNode,
) {
    'lines: for lineNumber in
        keyword.lineNumber+1..
            ({
                let mut scope = scopeJumps[keyword.lineNumber].clone();
                scope.reverse();
                root.GetNode(&mut scope).end
            })
    {
        let mut public = false;
        let mut currentContainer: Option <OutlineKeyword> = None;
        for (index, (token, text)) in tokenLines[lineNumber].iter().enumerate() {
            if text == "}" {  break 'lines;  }
            else if text == "pub" {  public = true;  }

            if matches!(token, TokenType::Comment | TokenType::String)
            {  continue;  }

            HandleScopeChecks(
                tokenLines,
                lineFlags,
                &mut currentContainer,
                scopeJumps,
                keyword,
                text,
                index,
                lineNumber,
                public
            );
        }

        if let Some(container) = currentContainer {
            newKeywords.push(container.clone());
            keyword.childKeywords.push(container);  // adding the variant
        }
    }
}

fn HandleScopeChecks (
    tokenLines: &[Vec <(TokenType, String)>],
    lineFlags: &Vec <Vec <Vec <LineTokenFlags>>>,
    currentContainer: &mut Option <OutlineKeyword>,
    scopeJumps: &Vec <Vec <usize>>,
    keyword: &mut OutlineKeyword,
    text: &String,
    index: usize,
    lineNumber: usize,
    public: bool
) {
    if lineFlags[lineNumber][index].contains(&LineTokenFlags::Parameter) && currentContainer.is_some() {
        HandleKeywordParameterSet(
            tokenLines,
            lineFlags,
            currentContainer,
            keyword,
            text,
            index,
            lineNumber
        );
    } else if !matches!(text.as_str(), " " | "(" | "Some" | "Ok" | "_" | "mut" | "," | "pub") && currentContainer.is_none() {
        let newKey = OutlineKeyword {
            keyword: text.clone(),
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
        currentContainer.replace(newKey);
    }
}

fn HandleKeywordParameterSet (
    tokenLines: &[Vec <(TokenType, String)>],
    lineFlags: &Vec <Vec <Vec <LineTokenFlags>>>,
    mut currentContainer: &mut Option <OutlineKeyword>,
    keyword: &mut OutlineKeyword,
    text: &String,
    index: usize,
    lineNumber: usize,
) {
    let mut parameterType = String::new();
    if text == "(" {
        HandleKeywordParameter(
            tokenLines,
            lineFlags,
            currentContainer,
            keyword,
            parameterType,
            index,
            lineNumber
        );
    } else if text == ":" {
        for newCharIndex in index + 2..tokenLines[lineNumber].len() {
            if lineFlags[lineNumber][index].contains(&LineTokenFlags::Comment) ||
                text == "," {  break;  }
            let string = &tokenLines[lineNumber][newCharIndex].1.clone();
            parameterType.push_str(string);
        }
        if let Some(container) = &mut currentContainer {
            container.parameters = Some(
                vec![(
                    tokenLines[lineNumber][index.saturating_sub(1)].1.clone(),
                    Some(parameterType)
                )]
            );
        }
    }
}

fn HandleKeywordParameter (
    tokenLines: &[Vec <(TokenType, String)>],
    lineFlags: &Vec <Vec <Vec <LineTokenFlags>>>,
    mut currentContainer: &mut Option <OutlineKeyword>,
    keyword: &mut OutlineKeyword,
    mut parameterType: String,
    index: usize,
    lineNumber: usize,
) {
    // getting the parameters type
    for newCharIndex in index + 1..tokenLines[lineNumber].len() {
        if !lineFlags[lineNumber][index].contains(&LineTokenFlags::Parameter) {
            break;
        }
        parameterType.push_str(&tokenLines[lineNumber][newCharIndex].1.clone());
    }
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
}

pub fn UpdateKeywordOutline (
    tokenLines: &[Vec <(TokenType, String)>],
    lineFlags: &Vec <Vec <Vec <LineTokenFlags>>>,
    outline: &mut Vec <OutlineKeyword>,
    scopeJumps: &Vec <Vec <usize>>,
    root: &ScopeNode
) {
    let implKeywordIndex = GetImplKeywordIndex(outline);
    HandleKeywordIndexes(outline, implKeywordIndex, scopeJumps, root);

    let mut newKeywords = HandleKeywords(tokenLines, lineFlags, outline, scopeJumps, root);
    while let Some(newKeyword) = newKeywords.pop() {
        outline.push(newKeyword);
    }
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
    tokenLines: &[Vec <(TokenType, String)>],
    lineFlags: &Vec <Vec <Vec <LineTokenFlags>>>,
    outline: &mut Vec <OutlineKeyword>
) -> (ScopeNode, Vec <Vec <usize>>, Vec <Vec <usize>>) {
    // tracking the scope (functions = new scope; struct/enums = new scope; for/while = new scope)
    let mut rootNode = ScopeNode {
        children: vec![],
        name: "Root".to_string(),
        start: 0,
        end: tokenLines.len().saturating_sub(1),
    };

    let mut jumps: Vec <Vec <usize>> = vec!();
    let mut linearized: Vec <Vec <usize>> = vec!();

    let mut currentScope: Vec <usize> = vec!();
    for (lineNumber, tokens) in tokenLines.iter().enumerate() {
        let mut bracketDepth = 0isize;
        // track the depth; if odd than based on the type of bracket add scope; if even do nothing (scope opened and closed on the same line)
        // use the same on functions to determine if the scope needs to continue or end on that line
        for (index, (token, name)) in tokens.iter().enumerate() {
            let (_lastToken, lastName) = {
                if index == 0 {  &(TokenType::Null, "".to_string())  }
                else {  &tokens[index - 1]  }
            };
            if !(matches!(token, TokenType::Comment) || (lastName == "\"" || lastName == "'") && matches!(token, TokenType::String)) {
                // checking bracket depth
                if name == "{" {
                    bracketDepth += 1;
                } else if name == "}" {
                    bracketDepth -= 1;
                }
            }

            // checking for something to define the name
            if matches!(token, TokenType::Keyword | TokenType::Object | TokenType::Function) {
                if !(VALID_NAMES_NEXT.contains(&name.trim()) || VALID_NAMES_TAKE.contains(&name.trim())) {
                    continue;
                }
                
                // checking the scope to see if ti continues or ends on the same line
                let mut brackDepth = 0isize;
                for (indx, (token, name)) in tokens.iter().enumerate() {
                    let (_lastToken, lastName) = {
                        if indx == 0 {  &(TokenType::Null, "".to_string())  }
                        else {  &tokens[indx - 1]  }
                    };
                    if !(matches!(token, TokenType::Comment) || lastName == "\"" && matches!(token, TokenType::String)) &&
                        indx > index 
                    {
                        if name == "{" {
                            brackDepth += 1;
                        } else if name == "}" {
                            brackDepth -= 1;
                        }
                    }
                }

                // check bracket-depth here so any overlapping marks are accounted for
                if bracketDepth < 0 && brackDepth > 0 {  // not pushing any jumps bc/ it'll be done later
                    let mut scopeCopy = currentScope.clone();
                    scopeCopy.reverse();
                    rootNode.SetEnd(&mut scopeCopy, lineNumber);
                    currentScope.pop();
                }

                // adding the new scope if necessary
                if brackDepth > 0 {
                    if VALID_NAMES_NEXT.contains(&name.trim()) {
                        let nextName = tokens.get(index + 2).unwrap_or(&(TokenType::Null, "".to_string())).1.clone();
                        
                        let mut scopeCopy = currentScope.clone();
                        scopeCopy.reverse();
                        let mut goodName = name.clone();
                        goodName.push(' ');
                        goodName.push_str(nextName.as_str());
                        let newScope = rootNode.Push(&mut scopeCopy, goodName, lineNumber);
                        currentScope.push(newScope);
                        linearized.push(currentScope.clone());
                        
                        bracketDepth = 0;
                        break;
                    } else if VALID_NAMES_TAKE.contains(&name.trim()) {
                        let mut scopeCopy = currentScope.clone();
                        scopeCopy.reverse();
                        let goodName = name.clone();
                        let newScope = rootNode.Push(&mut scopeCopy, goodName, lineNumber);
                        currentScope.push(newScope);
                        linearized.push(currentScope.clone());

                        bracketDepth = 0;
                        break;
                    }
                }
            }
        }
        
        // updating the scope based on the brackets
        if bracketDepth > 0 {
            let mut scopeCopy = currentScope.clone();
            scopeCopy.reverse();
            let newScope = rootNode.Push(&mut scopeCopy, "{ ... }".to_string(), lineNumber);
            currentScope.push(newScope);
            jumps.push(currentScope.clone());
            linearized.push(currentScope.clone());
            OutlineKeyword::EditScopes(outline, &currentScope, lineNumber);
        } else if bracketDepth < 0 {
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

    // updating the keywords outline
    UpdateKeywordOutline(tokenLines, lineFlags, outline, &jumps, &rootNode);

    (rootNode, jumps, linearized)
}

