
keywords = {"if", "for", "while", "in", "else", "break", "loop", "match",
            "return", "std", "const", "static", "dyn", "type", "continue",
            "use", "mod", "None", "Some", "Ok", "Err", "async", "await",
            "default", "derive", "as", "?", "ref", "allow", "where"}
primitives = {"i32", "isize", "i16", "i8", "i128", "i64", "u32", "usize",
              "u16", "u8", "u128", "u64", "f16", "f32", "f64", "f128",
              "String", "str", "Vec", "bool", "char", "Result", "Option",
              "Debug", "Clone", "Copy", "Default", "new"}
objects = {"enum", "pub", "struct", "impl", "self", "Self"}
mathLogicTokens = {"=", "<", ">", "!", "-", "+", "/", "*"}
logicTokens = {"=", "<", ">", "!"}
mathTokens = {"-", "+", "/", "*"}

-- checks if a value is in an array
function Contains (array, query)
    for index = 1, #array do
        if array[index] == query then
            return true
end end end


-- takes in a vector of strings   (GetTokens is the interfaced function w/ Rust)
function GetTokens (stringTokens)
    local parsedTokens = {}

    local inString = false;
    local inComment = false;

    local lastToken = ""
    local lastTokenType = "Null"

    -- going through the vector and parsing them
    for i, token in ipairs(stringTokens) do
        local nextToken = stringTokens[i + 1]
        local tokenType = "Null"

        -- handling multi-token flags
        if token == "\"" then
            inString = not inString
            tokenType = "String"
        elseif token == "/" and (lastToken == "/" or nextToken == "/") then
            inComment = true
            tokenType = "Comment"
        -- finding the token type
        elseif inString then
            tokenType = "String"
        elseif inComment then
            tokenType = "Comment"
        elseif token == " " then
            tokenType = "Null"
        elseif string.sub(token, 1, 1) == "_" then
            tokenType = "Grayed"
        else
            tokenType = ParseTokenType(lastTokenType, lastToken, nextToken, stringTokens[i + 2], token)
        end

        table.insert(parsedTokens, {tokenType, token})
        lastTokenType = tokenType
        lastToken = token
    end

    return parsedTokens
end

-- handles various extras
function ParseExtras (lastTokenType, lastToken, nextToken, token)
    if token == ">" or token == "<" or token == "!" then
        return "Logic"
    elseif token == "=" and Contains(logicTokens, lastToken) then
        return "Logic"
    elseif token == "&" and (nextToken == "&" or lastToken == "&") or token == "|" then
        return "Logic"
    elseif token == "&" then
        return "Barrow"
    elseif Contains(mathTokens, token) then
        return "Math"
    elseif token == "=" and (nextToken == "=" or Contains(mathTokens, lastToken)) then
        return "Math"
    elseif tonumber(string.sub(token, 1, 1)) ~= nil then
        return "Number"
    end

    return "Null"
end

-- checking for unsafe code
function Unchecked (token)
    if token == "unsafe" then
        return "Unsafe"
    end

    local splitText = {}
    for str in string.gmatch(token, '([^_]+)') do
        table.insert(splitText, str)
    end
    if #splitText == 2 then
        if splitText[2] == "unchecked" then
            return "Unsafe"
        end
    end

    return "Null"
end

-- parses basic tokens like brackets
function ParseBasic (lastTokenType, lastToken, nextToken, token)
    if token == "(" or token == ")" then
        return "Parentheses"
    elseif token == "[" or token == "]" then
        return "Bracket"
    elseif token == "{" or token == "}" or (
            token == "|" and nextToken ~= "|" and lastToken ~= "|"
    ) then
        return "SquirlyBracket"
    elseif token == ";" then
        return "Endl"
    elseif token == "let" or (
            token == "=" and not Contains(mathLogicTokens, lastToken) and
            nextToken ~= "="
    ) or token == "mut" then
        return "Assignment"
    elseif token == "fn" then
        return "Function"
    end

    return Unchecked(token)
end

-- does the more complex parts of token-parsing (not multi-token flags)
function ParseTokenType (lastTokenType, lastToken, nextToken, nextNextToken, token)
    -- parsing the basic tokens
    local tokenType = ParseBasic(lastTokenType, lastToken, nextToken, token)
    if tokenType ~= "Null" then
        return tokenType
    end

    -- checking for macros   parentheses and other basic characters should have already been weeded out
    if (token == "#") or (token == "!" and lastTokenType == "Macro") or (nextToken == "!") then
        return "Macro"
    end

    -- this needs the macros to be calculated but not the members, methods and objects
    tokenType = ParseExtras(lastTokenType, lastToken, nextToken, token)

    -- checking keywords & stuff
    if tokenType ~= "Null" then
        return tokenType
    end if Contains(keywords, token) then
        return "Keyword"
    elseif Contains(primitives, token) then
        return "Primitive"
    elseif Contains(objects, token) then
        return "Object"
    elseif token == ":" then
        return "Member"
    elseif lastToken == ":" or lastToken == "." then
        return CalculateMember(lastTokenType, lastToken, nextToken, token)
    else
        return ComplexTokens(lastTokenType, lastToken, nextToken, nextNextToken, token)
end end

-- calculating more complex tokens
function ComplexTokens (lastTokenType, lastToken, nextToken, nextNextToken, token)
    if token == "'" then
        if nextNextToken ~= "'" and lastTokenType ~= "String" and lastToken ~= " " then
            return "Lifetime"
        else
            return "String"
        end
    elseif lastToken == "'" and lastTokenType == "Lifetime" then
            return "Lifetime"
    elseif lastToken == "'" and nextToken == "'" then
        return "String"
    elseif string.upper(string.sub(token, 1, 1)) == string.sub(token, 1, 1) then
        return "Function"
    elseif string.upper(token) == token then
        return "Const"
    end

    return "Null"
end

-- calculating members/methods
function CalculateMember (lastTokenType, lastToken, nextToken, token)
    -- checking for a method
    local startingCharacter = string.sub(token, 1, 1)
    if string.upper(startingCharacter) == startingCharacter then
        return "Method"
    end

    return "Member"
end

