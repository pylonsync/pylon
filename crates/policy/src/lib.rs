use pylon_auth::AuthContext;
use pylon_kernel::{AppManifest, ManifestPolicy};

// ---------------------------------------------------------------------------
// Policy evaluation
// ---------------------------------------------------------------------------

/// Result of a policy check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyResult {
    Allowed,
    Denied { policy_name: String, reason: String },
}

/// Kind of entity access being checked. Drives which `allow_*` expression
/// the engine pulls from each manifest policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntityAction {
    Read,
    Insert,
    Update,
    Delete,
}

impl EntityAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Insert => "insert",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

impl PolicyResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, PolicyResult::Allowed)
    }
}

/// A policy engine that evaluates manifest policies against auth context.
///
/// Policy `allow` expressions are evaluated with simple pattern matching:
/// - `"auth.userId != null"` — requires authenticated user
/// - `"auth.userId == data.authorId"` — requires user matches data field
/// - `"auth.userId == input.authorId"` — requires user matches input field
/// - `"true"` — always allowed
///
/// This is NOT a full expression evaluator. It handles the common patterns
/// from the manifest contract. Complex expressions are treated as denied
/// with a clear message.
pub struct PolicyEngine {
    entity_policies: Vec<ManifestPolicy>,
    action_policies: Vec<ManifestPolicy>,
}

impl PolicyEngine {
    /// Build a policy engine from a manifest.
    pub fn from_manifest(manifest: &AppManifest) -> Self {
        let mut entity_policies = Vec::new();
        let mut action_policies = Vec::new();

        for policy in &manifest.policies {
            if policy.entity.is_some() {
                entity_policies.push(policy.clone());
            }
            if policy.action.is_some() {
                action_policies.push(policy.clone());
            }
        }

        Self {
            entity_policies,
            action_policies,
        }
    }

    /// Which kind of entity access is being checked. Lets the engine pick
    /// the most specific `allow_*` expression from a manifest policy and
    /// fall back through the override chain when no specific rule is set.
    fn expr_for<'a>(policy: &'a ManifestPolicy, action: EntityAction) -> &'a str {
        // Resolution order (most specific first):
        //   read   → allow_read                      → allow
        //   insert → allow_insert → allow_write      → allow
        //   update → allow_update → allow_write      → allow
        //   delete → allow_delete → allow_write      → allow
        //
        // An empty string means "no expression provided" → falls through.
        let pick = |primary: &'a Option<String>, secondary: &'a Option<String>| -> &'a str {
            if let Some(s) = primary.as_deref() {
                if !s.is_empty() {
                    return s;
                }
            }
            if let Some(s) = secondary.as_deref() {
                if !s.is_empty() {
                    return s;
                }
            }
            policy.allow.as_str()
        };
        match action {
            EntityAction::Read => pick(&policy.allow_read, &None),
            EntityAction::Insert => pick(&policy.allow_insert, &policy.allow_write),
            EntityAction::Update => pick(&policy.allow_update, &policy.allow_write),
            EntityAction::Delete => pick(&policy.allow_delete, &policy.allow_write),
        }
    }

    fn check_entity(
        &self,
        entity_name: &str,
        action: EntityAction,
        auth: &AuthContext,
        data: Option<&serde_json::Value>,
    ) -> PolicyResult {
        // Admin bypasses all policies.
        if auth.is_admin {
            return PolicyResult::Allowed;
        }

        let policies: Vec<&ManifestPolicy> = self
            .entity_policies
            .iter()
            .filter(|p| p.entity.as_deref() == Some(entity_name))
            .collect();

        if policies.is_empty() {
            return PolicyResult::Allowed;
        }

        for policy in &policies {
            let expr = Self::expr_for(policy, action);
            // Empty expression means "no rule at this level" — skip. A
            // policy without any applicable rule defers to the next
            // policy rather than silently denying.
            if expr.is_empty() {
                continue;
            }
            match evaluate_allow(expr, auth, data, None) {
                PolicyResult::Denied { .. } => {
                    return PolicyResult::Denied {
                        policy_name: policy.name.clone(),
                        reason: format!(
                            "Policy \"{}\" denied ({}): {}",
                            policy.name,
                            action.as_str(),
                            expr
                        ),
                    };
                }
                PolicyResult::Allowed => {}
            }
        }

        PolicyResult::Allowed
    }

    /// Check if an entity read is allowed for the given auth context.
    /// `data` is the row being accessed (for field-level checks).
    pub fn check_entity_read(
        &self,
        entity_name: &str,
        auth: &AuthContext,
        data: Option<&serde_json::Value>,
    ) -> PolicyResult {
        self.check_entity(entity_name, EntityAction::Read, auth, data)
    }

    /// Check if an entity write (insert/update/delete) is allowed.
    ///
    /// `data` is the incoming payload (for insert/update) or the existing row
    /// (for delete). Delegates to the specific insert/update/delete path
    /// when the caller knows the operation; kept as a generic entry point
    /// for legacy call sites that don't discriminate.
    pub fn check_entity_write(
        &self,
        entity_name: &str,
        auth: &AuthContext,
        data: Option<&serde_json::Value>,
    ) -> PolicyResult {
        self.check_entity(entity_name, EntityAction::Insert, auth, data)
    }

    /// Check if an entity insert is allowed. `data` is the incoming row.
    pub fn check_entity_insert(
        &self,
        entity_name: &str,
        auth: &AuthContext,
        data: Option<&serde_json::Value>,
    ) -> PolicyResult {
        self.check_entity(entity_name, EntityAction::Insert, auth, data)
    }

    /// Check if an entity update is allowed. `data` should be the existing
    /// row so ownership checks like `data.authorId == auth.userId` evaluate
    /// against truth instead of the incoming patch.
    pub fn check_entity_update(
        &self,
        entity_name: &str,
        auth: &AuthContext,
        data: Option<&serde_json::Value>,
    ) -> PolicyResult {
        self.check_entity(entity_name, EntityAction::Update, auth, data)
    }

    /// Check if an entity delete is allowed. `data` is the row about to be
    /// removed so delete-gates can look at the row's author/tenant fields.
    pub fn check_entity_delete(
        &self,
        entity_name: &str,
        auth: &AuthContext,
        data: Option<&serde_json::Value>,
    ) -> PolicyResult {
        self.check_entity(entity_name, EntityAction::Delete, auth, data)
    }

    /// Check if an action execution is allowed.
    /// `input` is the action input data.
    pub fn check_action(
        &self,
        action_name: &str,
        auth: &AuthContext,
        input: Option<&serde_json::Value>,
    ) -> PolicyResult {
        if auth.is_admin {
            return PolicyResult::Allowed;
        }

        let policies: Vec<&ManifestPolicy> = self
            .action_policies
            .iter()
            .filter(|p| p.action.as_deref() == Some(action_name))
            .collect();

        if policies.is_empty() {
            return PolicyResult::Allowed;
        }

        for policy in &policies {
            match evaluate_allow(&policy.allow, auth, None, input) {
                PolicyResult::Denied { .. } => {
                    return PolicyResult::Denied {
                        policy_name: policy.name.clone(),
                        reason: format!("Policy \"{}\" denied: {}", policy.name, policy.allow),
                    };
                }
                PolicyResult::Allowed => {}
            }
        }

        PolicyResult::Allowed
    }
}

/// Parse a comma-separated list of quoted strings (single or double quotes).
///
/// `"a", 'b,c', "d"` → `["a", "b,c", "d"]`. Respects quote boundaries so a
/// comma inside a quoted string is treated as part of the string, not a
/// separator. Used for `hasAnyRole` so role names containing commas aren't
/// silently split. Returns an error on unterminated strings or unquoted
/// tokens.
#[cfg(test)]
fn parse_quoted_string_list(s: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Skip whitespace and commas between items.
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let quote = bytes[i];
        if quote != b'"' && quote != b'\'' {
            return Err(format!(
                "expected quoted string at byte {i}, got {:?}",
                quote as char
            ));
        }
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        if i >= bytes.len() {
            return Err("unterminated quoted string".into());
        }
        let piece = &s[start..i];
        out.push(piece.to_string());
        i += 1; // skip closing quote
    }
    Ok(out)
}

/// Evaluate an `allow` expression against auth context and data.
///
/// Supports the following grammar (informal):
/// ```text
///   expr    := or
///   or      := and ("||" and)*
///   and     := not ("&&" not)*
///   not     := "!" not | primary
///   primary := "true" | "false"
///            | "(" expr ")"
///            | call
///            | path (("==" | "!=") atom)?
///   atom    := "null" | "true" | "false" | string | path
///   call    := "auth.hasRole" "(" string ")"
///            | "auth.hasAnyRole" "(" string ("," string)* ")"
///   path    := IDENT ("." IDENT)*    // auth.userId, data.author.id, etc.
/// ```
/// Existing primitives (`auth.userId != null`, `auth.isAdmin`,
/// `auth.hasRole(...)`, `auth.hasAnyRole(...)`, `auth.userId == data.<path>`)
/// are special cases of the grammar; old schemas keep working unchanged.
fn evaluate_allow(
    expr: &str,
    auth: &AuthContext,
    data: Option<&serde_json::Value>,
    input: Option<&serde_json::Value>,
) -> PolicyResult {
    let tokens = match tokenize(expr) {
        Ok(t) => t,
        Err(e) => {
            return PolicyResult::Denied {
                policy_name: String::new(),
                reason: format!("Policy parse error: {e} (in {expr:?})"),
            };
        }
    };
    let mut parser = Parser::new(&tokens);
    let ast = match parser.parse_expr() {
        Ok(a) => a,
        Err(e) => {
            return PolicyResult::Denied {
                policy_name: String::new(),
                reason: format!("Policy parse error: {e} (in {expr:?})"),
            };
        }
    };
    if !parser.at_end() {
        return PolicyResult::Denied {
            policy_name: String::new(),
            reason: format!("Trailing tokens in expression: {expr:?}"),
        };
    }
    let env = EvalEnv { auth, data, input };
    match env.eval(&ast) {
        EvalResult::True => PolicyResult::Allowed,
        EvalResult::False(reason) => PolicyResult::Denied {
            policy_name: String::new(),
            reason,
        },
    }
}

// ---------------------------------------------------------------------------
// Expression parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    True,
    False,
    Null,
    And, // &&
    Or,  // ||
    Not, // !
    Eq,  // ==
    Neq, // !=
    LParen,
    RParen,
    Comma,
    Ident(String),
    Str(String),
}

fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b' ' | b'\t' | b'\n' | b'\r' => {
                i += 1;
            }
            b'(' => {
                out.push(Token::LParen);
                i += 1;
            }
            b')' => {
                out.push(Token::RParen);
                i += 1;
            }
            b',' => {
                out.push(Token::Comma);
                i += 1;
            }
            b'&' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'&' {
                    out.push(Token::And);
                    i += 2;
                } else {
                    return Err("single `&` — did you mean `&&`?".into());
                }
            }
            b'|' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                    out.push(Token::Or);
                    i += 2;
                } else {
                    return Err("single `|` — did you mean `||`?".into());
                }
            }
            b'=' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    out.push(Token::Eq);
                    i += 2;
                } else {
                    return Err("single `=` — did you mean `==`?".into());
                }
            }
            b'!' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'=' {
                    out.push(Token::Neq);
                    i += 2;
                } else {
                    out.push(Token::Not);
                    i += 1;
                }
            }
            b'"' | b'\'' => {
                // Parse the literal as chars (not bytes) so multi-byte UTF-8
                // round-trips intact. Previously `unescaped.push(b as char)`
                // mangled anything outside ASCII: `"é"` became two garbage
                // chars. Only a fixed escape set is honored; unknown escapes
                // now error rather than silently dropping the backslash
                // (old behavior turned `"\n"` into `"n"`).
                let quote = c as char;
                // Skip opening quote, then walk the rest of the string as
                // a char iterator. Build `unescaped` directly — we don't
                // need a raw slice anymore since escapes are resolved inline.
                let rest = &src[i + 1..];
                let mut chars = rest.char_indices();
                let mut unescaped = String::new();
                let mut closed_at: Option<usize> = None;
                while let Some((rel, ch)) = chars.next() {
                    if ch == quote {
                        closed_at = Some(i + 1 + rel + ch.len_utf8());
                        break;
                    }
                    if ch == '\\' {
                        let (_rel2, esc) = chars
                            .next()
                            .ok_or_else(|| "unterminated string literal".to_string())?;
                        match esc {
                            '\\' => unescaped.push('\\'),
                            '"' => unescaped.push('"'),
                            '\'' => unescaped.push('\''),
                            'n' => unescaped.push('\n'),
                            'r' => unescaped.push('\r'),
                            't' => unescaped.push('\t'),
                            '0' => unescaped.push('\0'),
                            other => {
                                return Err(format!("unknown string escape `\\{other}`"));
                            }
                        }
                    } else {
                        unescaped.push(ch);
                    }
                }
                let close = closed_at.ok_or_else(|| "unterminated string literal".to_string())?;
                out.push(Token::Str(unescaped));
                i = close;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < bytes.len() {
                    let ch = bytes[i];
                    if ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'.' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                let word = &src[start..i];
                match word {
                    "true" => out.push(Token::True),
                    "false" => out.push(Token::False),
                    "null" => out.push(Token::Null),
                    _ => out.push(Token::Ident(word.to_string())),
                }
            }
            other => {
                return Err(format!("unexpected character {:?}", other as char));
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
enum Ast {
    True,
    False,
    Not(Box<Ast>),
    And(Box<Ast>, Box<Ast>),
    Or(Box<Ast>, Box<Ast>),
    Eq(Box<Ast>, Box<Ast>),
    Neq(Box<Ast>, Box<Ast>),
    /// `auth.hasRole("x")`
    HasRole(String),
    /// `auth.hasAnyRole("a", "b", ...)`
    HasAnyRole(Vec<String>),
    /// A path like `auth.userId` or `data.author.id`.
    Path(Vec<String>),
    /// A string literal.
    Str(String),
    /// `null` literal.
    Null,
    /// Degenerate: bare `auth.isAdmin` etc. resolves to a boolean.
    Bool(bool),
}

/// Cap recursive descent so a pathological input like `((((...!x))))` can't
/// stack-overflow the server thread. 64 is far beyond any realistic policy —
/// a hand-authored expression rarely nests more than 3–4 levels.
const MAX_PARSE_DEPTH: usize = 64;

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            pos: 0,
            depth: 0,
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<&Token> {
        let t = self.tokens.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// Enter one level of recursion, erroring if we exceed the cap.
    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_PARSE_DEPTH {
            return Err(format!(
                "policy expression nested deeper than {MAX_PARSE_DEPTH} levels"
            ));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn parse_expr(&mut self) -> Result<Ast, String> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Ast, String> {
        self.enter()?;
        let mut lhs = self.parse_and()?;
        while matches!(self.peek(), Some(Token::Or)) {
            self.bump();
            let rhs = self.parse_and()?;
            lhs = Ast::Or(Box::new(lhs), Box::new(rhs));
        }
        self.leave();
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Ast, String> {
        let mut lhs = self.parse_comparison()?;
        while matches!(self.peek(), Some(Token::And)) {
            self.bump();
            let rhs = self.parse_comparison()?;
            lhs = Ast::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// Comparison binds LOOSER than `!`, so `!x == null` parses as
    /// `(!x) == null` — matching conventional precedence in languages like
    /// JS/Rust. Previously `parse_primary` ate `== null` greedily, causing
    /// `!x == null` to evaluate as `!(x == null)` which is almost never
    /// what a rule author intends.
    fn parse_comparison(&mut self) -> Result<Ast, String> {
        let lhs = self.parse_not()?;
        match self.peek() {
            Some(Token::Eq) => {
                self.bump();
                let rhs = self.parse_atom()?;
                Ok(Ast::Eq(Box::new(lhs), Box::new(rhs)))
            }
            Some(Token::Neq) => {
                self.bump();
                let rhs = self.parse_atom()?;
                Ok(Ast::Neq(Box::new(lhs), Box::new(rhs)))
            }
            _ => Ok(lhs),
        }
    }

    fn parse_not(&mut self) -> Result<Ast, String> {
        if matches!(self.peek(), Some(Token::Not)) {
            self.bump();
            self.enter()?;
            let inner = self.parse_not()?;
            self.leave();
            return Ok(Ast::Not(Box::new(inner)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Ast, String> {
        match self.peek().cloned() {
            Some(Token::True) => {
                self.bump();
                Ok(Ast::True)
            }
            Some(Token::False) => {
                self.bump();
                Ok(Ast::False)
            }
            Some(Token::Null) => {
                self.bump();
                Ok(Ast::Null)
            }
            Some(Token::Str(s)) => {
                self.bump();
                Ok(Ast::Str(s))
            }
            Some(Token::LParen) => {
                self.bump();
                self.enter()?;
                let inner = self.parse_expr()?;
                self.leave();
                match self.peek() {
                    Some(Token::RParen) => {
                        self.bump();
                    }
                    _ => return Err("expected `)`".into()),
                }
                Ok(inner)
            }
            Some(Token::Ident(name)) => {
                self.bump();
                // Two cases: path, or function call.
                if matches!(self.peek(), Some(Token::LParen)) {
                    // Function call. Only two functions are built in.
                    self.bump();
                    let args = self.parse_string_args()?;
                    match self.peek() {
                        Some(Token::RParen) => {
                            self.bump();
                        }
                        _ => return Err("expected `)` after function args".into()),
                    }
                    return self.build_call(&name, args);
                }
                // Comparison (==, !=) is handled by parse_comparison above —
                // intentionally NOT consumed here, so `!x == null` parses as
                // `(!x) == null` instead of `!(x == null)`.
                Ok(Ast::Path(split_path(&name)))
            }
            Some(other) => Err(format!("unexpected token {other:?}")),
            None => Err("unexpected end of expression".into()),
        }
    }

    fn parse_string_args(&mut self) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        loop {
            match self.peek().cloned() {
                Some(Token::Str(s)) => {
                    self.bump();
                    out.push(s);
                }
                Some(Token::RParen) => break,
                Some(other) => {
                    return Err(format!("expected quoted string argument, got {other:?}"));
                }
                None => return Err("unexpected end inside function args".into()),
            }
            match self.peek() {
                Some(Token::Comma) => {
                    self.bump();
                }
                Some(Token::RParen) => break,
                _ => break,
            }
        }
        Ok(out)
    }

    fn build_call(&mut self, name: &str, args: Vec<String>) -> Result<Ast, String> {
        match name {
            "auth.hasRole" => {
                if args.len() != 1 {
                    return Err("auth.hasRole takes exactly one string argument".into());
                }
                Ok(Ast::HasRole(args.into_iter().next().unwrap()))
            }
            "auth.hasAnyRole" => {
                if args.is_empty() {
                    return Err("auth.hasAnyRole takes at least one argument".into());
                }
                Ok(Ast::HasAnyRole(args))
            }
            other => Err(format!("unknown function \"{other}(...)\"")),
        }
    }

    fn parse_atom(&mut self) -> Result<Ast, String> {
        match self.peek().cloned() {
            Some(Token::Null) => {
                self.bump();
                Ok(Ast::Null)
            }
            Some(Token::True) => {
                self.bump();
                Ok(Ast::Bool(true))
            }
            Some(Token::False) => {
                self.bump();
                Ok(Ast::Bool(false))
            }
            Some(Token::Str(s)) => {
                self.bump();
                Ok(Ast::Str(s))
            }
            Some(Token::Ident(name)) => {
                self.bump();
                Ok(Ast::Path(split_path(&name)))
            }
            Some(other) => Err(format!("expected atom, got {other:?}")),
            None => Err("unexpected end of expression in atom".into()),
        }
    }
}

fn split_path(s: &str) -> Vec<String> {
    s.split('.').map(|p| p.to_string()).collect()
}

struct EvalEnv<'a> {
    auth: &'a AuthContext,
    data: Option<&'a serde_json::Value>,
    input: Option<&'a serde_json::Value>,
}

#[derive(Debug)]
enum EvalResult {
    True,
    False(String),
}

#[derive(Debug, Clone)]
enum Value {
    Str(String),
    Bool(bool),
    Null,
}

impl<'a> EvalEnv<'a> {
    fn eval(&self, ast: &Ast) -> EvalResult {
        match ast {
            Ast::True => EvalResult::True,
            Ast::False => EvalResult::False("Expression is false".into()),
            Ast::Not(inner) => match self.eval(inner) {
                EvalResult::True => EvalResult::False("Negated expression was true".into()),
                EvalResult::False(_) => EvalResult::True,
            },
            Ast::And(l, r) => match self.eval(l) {
                EvalResult::False(reason) => EvalResult::False(reason),
                EvalResult::True => self.eval(r),
            },
            Ast::Or(l, r) => match self.eval(l) {
                EvalResult::True => EvalResult::True,
                EvalResult::False(reason_l) => match self.eval(r) {
                    EvalResult::True => EvalResult::True,
                    EvalResult::False(reason_r) => {
                        EvalResult::False(format!("{reason_l}; and {reason_r}"))
                    }
                },
            },
            Ast::Eq(l, r) => {
                let lv = self.value_of(l);
                let rv = self.value_of(r);
                if values_eq(&lv, &rv) {
                    EvalResult::True
                } else {
                    EvalResult::False(format!("{lv:?} != {rv:?}"))
                }
            }
            Ast::Neq(l, r) => {
                let lv = self.value_of(l);
                let rv = self.value_of(r);
                if values_eq(&lv, &rv) {
                    EvalResult::False(format!("{lv:?} == {rv:?}"))
                } else {
                    EvalResult::True
                }
            }
            Ast::HasRole(role) => {
                if self.auth.has_role(role) {
                    EvalResult::True
                } else {
                    EvalResult::False(format!("Missing required role \"{role}\""))
                }
            }
            Ast::HasAnyRole(roles) => {
                let refs: Vec<&str> = roles.iter().map(|s| s.as_str()).collect();
                if self.auth.has_any_role(&refs) {
                    EvalResult::True
                } else {
                    EvalResult::False(format!("Missing any of required roles: {refs:?}"))
                }
            }
            Ast::Path(_) | Ast::Str(_) | Ast::Null | Ast::Bool(_) => {
                // Bare value as boolean expression.
                match self.value_of(ast) {
                    Value::Bool(true) => EvalResult::True,
                    Value::Bool(false) => EvalResult::False("Expression evaluated to false".into()),
                    Value::Null => EvalResult::False("Expression evaluated to null".into()),
                    Value::Str(s) => {
                        // Non-empty string is truthy (matches JS-ish intuition).
                        if s.is_empty() {
                            EvalResult::False("Empty string".into())
                        } else {
                            EvalResult::True
                        }
                    }
                }
            }
        }
    }

    fn value_of(&self, ast: &Ast) -> Value {
        match ast {
            Ast::Null => Value::Null,
            Ast::Str(s) => Value::Str(s.clone()),
            Ast::Bool(b) => Value::Bool(*b),
            Ast::Path(parts) => self.resolve_path(parts),
            // Nested boolean ops evaluate to Bool.
            other => match self.eval(other) {
                EvalResult::True => Value::Bool(true),
                EvalResult::False(_) => Value::Bool(false),
            },
        }
    }

    fn resolve_path(&self, parts: &[String]) -> Value {
        if parts.is_empty() {
            return Value::Null;
        }
        match parts[0].as_str() {
            "auth" => self.resolve_auth(&parts[1..]),
            "data" => self.resolve_json(self.data, &parts[1..]),
            "input" => self.resolve_json(self.input, &parts[1..]),
            other => {
                // Unknown top-level — treat as null so policies fail closed
                // rather than authorizing based on unresolved identifiers.
                let _ = other;
                Value::Null
            }
        }
    }

    fn resolve_auth(&self, parts: &[String]) -> Value {
        // `auth.<field>` must name EXACTLY one field. Previously trailing
        // segments like `auth.isAdmin.foo` or `auth.userId.x.y` silently
        // resolved to the base field, over-broadening the allowed paths
        // and masking typos. Require len == 1.
        if parts.len() != 1 {
            return Value::Null;
        }
        match parts[0].as_str() {
            "userId" | "user_id" => match &self.auth.user_id {
                Some(s) => Value::Str(s.clone()),
                None => Value::Null,
            },
            "isAdmin" | "is_admin" => Value::Bool(self.auth.is_admin),
            "tenantId" | "tenant_id" => match &self.auth.tenant_id {
                Some(s) => Value::Str(s.clone()),
                None => Value::Null,
            },
            _ => Value::Null,
        }
    }

    fn resolve_json(&self, root: Option<&serde_json::Value>, parts: &[String]) -> Value {
        let mut cur = match root {
            Some(v) => v,
            None => return Value::Null,
        };
        for p in parts {
            cur = match cur.get(p) {
                Some(v) => v,
                None => return Value::Null,
            };
        }
        match cur {
            serde_json::Value::String(s) => Value::Str(s.clone()),
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Number(n) => Value::Str(n.to_string()),
            _ => Value::Null,
        }
    }
}

fn values_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        // Mixed types are never equal (no coercion).
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::ManifestPolicy;

    // -----------------------------------------------------------------------
    // New expression grammar: &&, ||, !, parens, nested paths
    // -----------------------------------------------------------------------

    fn alice_owns(post_author: &str) -> (AuthContext, serde_json::Value) {
        let auth = AuthContext::authenticated("alice".into());
        let data = serde_json::json!({ "authorId": post_author, "status": "draft" });
        (auth, data)
    }

    #[test]
    fn conjunction_needs_both_sides() {
        let (auth, data) = alice_owns("alice");
        let r = evaluate_allow(
            "auth.userId != null && auth.userId == data.authorId",
            &auth,
            Some(&data),
            None,
        );
        assert!(matches!(r, PolicyResult::Allowed));
    }

    #[test]
    fn conjunction_fails_when_either_fails() {
        let (auth, data) = alice_owns("bob"); // not alice
        let r = evaluate_allow(
            "auth.userId != null && auth.userId == data.authorId",
            &auth,
            Some(&data),
            None,
        );
        assert!(!r.is_allowed());
    }

    #[test]
    fn disjunction_allows_admin_or_owner() {
        // Non-admin authed user; data owner is alice; check passes via owner.
        let (auth, data) = alice_owns("alice");
        let r = evaluate_allow(
            "auth.isAdmin || auth.userId == data.authorId",
            &auth,
            Some(&data),
            None,
        );
        assert!(matches!(r, PolicyResult::Allowed));

        // Admin short-circuits even when not the owner.
        let admin = AuthContext::admin();
        let r2 = evaluate_allow(
            "auth.isAdmin || auth.userId == data.authorId",
            &admin,
            Some(&data),
            None,
        );
        assert!(matches!(r2, PolicyResult::Allowed));
    }

    #[test]
    fn negation_inverts_bool() {
        let auth = AuthContext::anonymous();
        let r = evaluate_allow("!auth.isAdmin", &auth, None, None);
        assert!(matches!(r, PolicyResult::Allowed));

        let admin = AuthContext::admin();
        let r2 = evaluate_allow("!auth.isAdmin", &admin, None, None);
        assert!(!r2.is_allowed());
    }

    #[test]
    fn parentheses_group_correctly() {
        let auth = AuthContext::anonymous();
        let data = serde_json::json!({ "public": true });
        // Should evaluate as: admin OR (authed AND public)
        let expr = "auth.isAdmin || (auth.userId != null && data.public == true)";
        assert!(!evaluate_allow(expr, &auth, Some(&data), None).is_allowed());

        let authed = AuthContext::authenticated("alice".into());
        assert!(evaluate_allow(expr, &authed, Some(&data), None).is_allowed());
    }

    #[test]
    fn nested_data_path() {
        let auth = AuthContext::authenticated("alice".into());
        let data = serde_json::json!({ "author": { "id": "alice" } });
        assert!(
            evaluate_allow("auth.userId == data.author.id", &auth, Some(&data), None).is_allowed()
        );
    }

    #[test]
    fn null_comparison() {
        let auth = AuthContext::authenticated("alice".into());
        let data = serde_json::json!({ "deletedAt": null });
        assert!(evaluate_allow("data.deletedAt == null", &auth, Some(&data), None).is_allowed());
    }

    #[test]
    fn string_literal_equality() {
        let auth = AuthContext::authenticated("alice".into());
        let data = serde_json::json!({ "status": "published" });
        assert!(
            evaluate_allow("data.status == \"published\"", &auth, Some(&data), None).is_allowed()
        );
        assert!(!evaluate_allow("data.status == \"draft\"", &auth, Some(&data), None).is_allowed());
    }

    #[test]
    fn tenant_predicate() {
        let auth = AuthContext::authenticated("alice".into()).with_tenant("acme".into());
        let data = serde_json::json!({ "tenantId": "acme" });
        assert!(
            evaluate_allow("auth.tenantId == data.tenantId", &auth, Some(&data), None).is_allowed()
        );
        let data2 = serde_json::json!({ "tenantId": "other" });
        assert!(
            !evaluate_allow("auth.tenantId == data.tenantId", &auth, Some(&data2), None)
                .is_allowed()
        );
    }

    #[test]
    fn malformed_expression_denies_closed() {
        let auth = AuthContext::admin();
        let r = evaluate_allow("auth.userId == ", &auth, None, None);
        assert!(!r.is_allowed(), "parse error must fail closed");
    }

    #[test]
    fn unknown_identifier_resolves_to_null() {
        // Fail-closed: an unknown top-level identifier becomes null, so a
        // comparison against anything non-null is false.
        let auth = AuthContext::admin();
        let r = evaluate_allow("zzz.field == \"x\"", &auth, None, None);
        assert!(!r.is_allowed());
    }

    // -----------------------------------------------------------------------
    // Regression tests from the 2026 policy review.
    // -----------------------------------------------------------------------

    #[test]
    fn string_escape_n_is_newline() {
        // Prior bug: byte-wise unescape turned `\n` into the letter `n`.
        // Now the scanner honors the standard escape set and preserves UTF-8.
        let auth = AuthContext::anonymous();
        let data = serde_json::json!({ "note": "line1\nline2" });
        assert!(
            evaluate_allow("data.note == \"line1\\nline2\"", &auth, Some(&data), None).is_allowed()
        );
    }

    #[test]
    fn string_escape_unknown_is_error() {
        // Previously `\q` silently collapsed to `q`. Now it's a parse error
        // that fails closed — authors get loud feedback instead of a
        // subtly-wrong rule.
        let auth = AuthContext::anonymous();
        let r = evaluate_allow("data.x == \"\\q\"", &auth, None, None);
        assert!(!r.is_allowed());
    }

    #[test]
    fn string_literal_preserves_utf8() {
        // Prior bug: `unescaped.push(b as char)` mangled `é` into garbage.
        let auth = AuthContext::anonymous();
        let data = serde_json::json!({ "name": "café" });
        assert!(evaluate_allow("data.name == \"café\"", &auth, Some(&data), None).is_allowed());
    }

    #[test]
    fn not_precedence_binds_tighter_than_eq() {
        // Prior bug: `!auth.isAdmin == false` parsed as `!(auth.isAdmin == false)`,
        // so an anonymous caller (whose isAdmin is false) would be DENIED
        // because !(false == false) == !(true) == false. With correct
        // precedence `(!auth.isAdmin) == false` is (!false) == false == true
        // only when isAdmin is true.
        let anon = AuthContext::anonymous();
        let admin = AuthContext::admin();
        // For anonymous: (!false) == false  ->  true == false -> false
        let r = evaluate_allow("!auth.isAdmin == false", &anon, None, None);
        assert!(!r.is_allowed(), "anon: (!false) == false should be false");
        // For admin: (!true) == false  ->  false == false -> true
        let r2 = evaluate_allow("!auth.isAdmin == false", &admin, None, None);
        assert!(r2.is_allowed(), "admin: (!true) == false should be true");
    }

    #[test]
    fn auth_path_rejects_extra_segments() {
        // Prior bug: `auth.isAdmin.foo` resolved as if it were `auth.isAdmin`
        // because `resolve_auth` only looked at the first segment. Now extra
        // segments return Null, which makes the comparison false.
        let admin = AuthContext::admin();
        let r = evaluate_allow("auth.isAdmin.foo == true", &admin, None, None);
        assert!(!r.is_allowed(), "extra segment must resolve to null");
        let r2 = evaluate_allow("auth.userId.x == \"anyone\"", &admin, None, None);
        assert!(!r2.is_allowed());
    }

    #[test]
    fn deep_nesting_rejected_not_panicking() {
        // Prior bug: no depth cap; 10_000 parens would stack-overflow.
        // Now the parser returns an error well before that, and
        // evaluate_allow converts it to Denied.
        let auth = AuthContext::anonymous();
        let expr = format!("{}true{}", "(".repeat(200), ")".repeat(200));
        let r = evaluate_allow(&expr, &auth, None, None);
        assert!(!r.is_allowed(), "deep nesting must deny closed, not panic");
    }

    #[test]
    fn moderate_nesting_still_parses() {
        // The cap must not break realistic expressions. 10 levels is fine.
        let auth = AuthContext::anonymous();
        let expr = format!("{}true{}", "(".repeat(10), ")".repeat(10));
        assert!(evaluate_allow(&expr, &auth, None, None).is_allowed());
    }

    #[test]
    fn parse_quoted_list_single_role() {
        assert_eq!(
            parse_quoted_string_list("\"admin\"").unwrap(),
            vec!["admin"]
        );
    }

    #[test]
    fn parse_quoted_list_two_roles() {
        assert_eq!(
            parse_quoted_string_list("'billing', 'admin'").unwrap(),
            vec!["billing", "admin"]
        );
    }

    #[test]
    fn parse_quoted_list_comma_inside_string_is_literal() {
        // This is the whole point of the fix.
        assert_eq!(
            parse_quoted_string_list("\"billing,admin\"").unwrap(),
            vec!["billing,admin"]
        );
    }

    #[test]
    fn parse_quoted_list_rejects_unquoted() {
        assert!(parse_quoted_string_list("admin").is_err());
    }

    #[test]
    fn parse_quoted_list_rejects_unterminated() {
        assert!(parse_quoted_string_list("\"unterminated").is_err());
    }

    fn test_manifest() -> AppManifest {
        serde_json::from_str(include_str!(
            "../../../examples/todo-app/pylon.manifest.json"
        ))
        .unwrap()
    }

    #[test]
    fn engine_from_manifest() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        assert_eq!(engine.entity_policies.len(), 1); // ownerReadTodos
        assert_eq!(engine.action_policies.len(), 2); // authenticatedCreate, ownerToggle
    }

    #[test]
    fn no_policies_allows_access() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        let auth = AuthContext::anonymous();
        // User entity has no policies.
        let result = engine.check_entity_read("User", &auth, None);
        assert!(result.is_allowed());
    }

    #[test]
    fn auth_required_denies_anonymous() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        let auth = AuthContext::anonymous();
        let result = engine.check_action("createTodo", &auth, None);
        assert!(!result.is_allowed());
    }

    #[test]
    fn auth_required_allows_authenticated() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        let auth = AuthContext::authenticated("user-1".into());
        let result = engine.check_action("createTodo", &auth, None);
        assert!(result.is_allowed());
    }

    #[test]
    fn owner_check_on_entity() {
        let engine = PolicyEngine::from_manifest(&test_manifest());

        // Owner access allowed.
        let auth = AuthContext::authenticated("user-1".into());
        let data = serde_json::json!({"authorId": "user-1"});
        let result = engine.check_entity_read("Todo", &auth, Some(&data));
        assert!(result.is_allowed());

        // Non-owner denied.
        let auth = AuthContext::authenticated("user-2".into());
        let result = engine.check_entity_read("Todo", &auth, Some(&data));
        assert!(!result.is_allowed());
    }

    #[test]
    fn owner_check_on_action_input() {
        let engine = PolicyEngine::from_manifest(&test_manifest());

        // toggleTodo requires auth.userId == input.authorId
        let auth = AuthContext::authenticated("user-1".into());
        let input = serde_json::json!({"authorId": "user-1", "todoId": "todo-1"});
        let result = engine.check_action("toggleTodo", &auth, Some(&input));
        assert!(result.is_allowed());

        let auth = AuthContext::authenticated("user-2".into());
        let result = engine.check_action("toggleTodo", &auth, Some(&input));
        assert!(!result.is_allowed());
    }

    #[test]
    fn true_expression_always_allows() {
        let result = evaluate_allow("true", &AuthContext::anonymous(), None, None);
        assert!(result.is_allowed());
    }

    #[test]
    fn false_expression_always_denies() {
        let result = evaluate_allow("false", &AuthContext::anonymous(), None, None);
        assert!(!result.is_allowed());
    }

    #[test]
    fn unknown_expression_denies() {
        let result = evaluate_allow(
            "some.complex.expression",
            &AuthContext::anonymous(),
            None,
            None,
        );
        assert!(!result.is_allowed());
    }

    // -- Admin bypass --

    #[test]
    fn admin_bypasses_entity_policy() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        let admin = AuthContext::admin();
        let result = engine.check_entity_read("Todo", &admin, None);
        assert!(result.is_allowed());
    }

    #[test]
    fn admin_bypasses_action_policy() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        let admin = AuthContext::admin();
        let result = engine.check_action("createTodo", &admin, None);
        assert!(result.is_allowed());
    }

    #[test]
    fn non_admin_still_denied() {
        let engine = PolicyEngine::from_manifest(&test_manifest());
        let anon = AuthContext::anonymous();
        let result = engine.check_action("createTodo", &anon, None);
        assert!(!result.is_allowed());
    }

    // -- Expression edge cases --

    #[test]
    fn data_field_check_without_data() {
        let result = evaluate_allow(
            "auth.userId == data.authorId",
            &AuthContext::authenticated("user-1".into()),
            None, // no data
            None,
        );
        assert!(!result.is_allowed());
    }

    #[test]
    fn input_field_check_without_input() {
        let result = evaluate_allow(
            "auth.userId == input.authorId",
            &AuthContext::authenticated("user-1".into()),
            None,
            None, // no input
        );
        assert!(!result.is_allowed());
    }

    #[test]
    fn data_field_user_mismatch() {
        let data = serde_json::json!({"authorId": "other-user"});
        let result = evaluate_allow(
            "auth.userId == data.authorId",
            &AuthContext::authenticated("user-1".into()),
            Some(&data),
            None,
        );
        assert!(!result.is_allowed());
    }

    #[test]
    fn input_field_user_mismatch() {
        let input = serde_json::json!({"authorId": "other-user"});
        let result = evaluate_allow(
            "auth.userId == input.authorId",
            &AuthContext::authenticated("user-1".into()),
            None,
            Some(&input),
        );
        assert!(!result.is_allowed());
    }

    #[test]
    fn data_field_anonymous_denied() {
        let data = serde_json::json!({"authorId": "user-1"});
        let result = evaluate_allow(
            "auth.userId == data.authorId",
            &AuthContext::anonymous(),
            Some(&data),
            None,
        );
        assert!(!result.is_allowed());
    }
}
