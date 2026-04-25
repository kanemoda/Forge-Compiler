//! Symbol table and lexical scopes.
//!
//! C17 §6.2.3 defines four disjoint namespaces for identifiers:
//!
//! 1. **Labels** — owned by the enclosing function; tracked elsewhere.
//! 2. **Tags** — `struct` / `union` / `enum` names; one per [`Scope`].
//! 3. **Members** — per-compound-type; stored on the struct layout.
//! 4. **Ordinary** — variables, functions, typedefs, enum constants,
//!    and function parameters; one per [`Scope`].
//!
//! This module covers the *tag* and *ordinary* namespaces — the two
//! that live on lexical scopes.  Labels and members are owned by their
//! containing structures and sit elsewhere in the sema crate.
//!
//! The outermost [`Scope`] is always [`ScopeKind::File`]; function
//! bodies introduce a [`ScopeKind::Function`] scope; compound
//! statements and `for` init clauses push [`ScopeKind::Block`] scopes;
//! prototype scopes are pushed only while resolving a function
//! declarator's parameter list.

use std::collections::HashMap;

use forge_diagnostics::Diagnostic;
use forge_lexer::Span;

use crate::context::SemaContext;
use crate::types::{
    are_compatible, composite_type, EnumTypeId, QualType, StructTypeId, UnionTypeId,
};

// =========================================================================
// Id newtypes
// =========================================================================

/// Dense identifier for a symbol-table entry.  Stable across scope
/// pushes and pops for the lifetime of a [`SymbolTable`].
pub type SymbolId = u32;

/// Dense identifier for a tag entry (struct / union / enum).
pub type TagId = u32;

// =========================================================================
// Kinds
// =========================================================================

/// What kind of entity a symbol represents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    /// A variable: `int x;`.
    Variable,
    /// A function with an accompanying declarator.
    Function,
    /// `typedef T N;`.
    Typedef,
    /// An enumerator inside an `enum` body.
    EnumConstant {
        /// The enumerator's integer value.
        value: i64,
        /// The enum type it belongs to.
        enum_id: EnumTypeId,
    },
    /// A function parameter introduced by a prototype or function
    /// definition.
    Parameter,
}

/// C17 storage-class specifier, expanded with an explicit `None` for
/// declarations that omit one (the default on ordinary locals /
/// function declarations).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StorageClass {
    /// No storage class written.  Default for most declarations.
    None,
    /// `auto`
    Auto,
    /// `register`
    Register,
    /// `static`
    Static,
    /// `extern`
    Extern,
    /// `_Thread_local`
    ThreadLocal,
}

/// C17 §6.2.2 linkage kind — determined by storage class and scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Linkage {
    /// Visible only within the enclosing block (no linkage at all).
    None,
    /// File-scope identifiers with `static`.
    Internal,
    /// File-scope identifiers without `static`, or `extern`.
    External,
}

/// What lexical category does a scope belong to?
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScopeKind {
    /// The top-level translation-unit scope.
    File,
    /// The function-wide scope (used for parameters and labels).
    Function,
    /// A compound statement or a `for` init clause.
    Block,
    /// Temporary scope used while resolving a function declarator's
    /// parameter list.  Symbols declared here are not visible outside
    /// the declarator.
    Prototype,
}

// =========================================================================
// Symbol
// =========================================================================

/// An entry in the ordinary namespace.
#[derive(Clone, Debug)]
pub struct Symbol {
    /// Dense id assigned on insertion.
    pub id: SymbolId,
    /// The identifier spelling.
    pub name: String,
    /// Resolved qualified type.
    pub ty: QualType,
    /// What kind of entity this is.
    pub kind: SymbolKind,
    /// Storage class of the declaration.
    pub storage: StorageClass,
    /// Linkage derived from scope + storage class.
    pub linkage: Linkage,
    /// Span of the declaring identifier.
    pub span: Span,
    /// `true` if this declaration provides a definition (body for a
    /// function, initializer or non-tentative def for an object).
    pub is_defined: bool,
    /// C17 §6.7.4 `inline` function specifier.  Only meaningful on
    /// [`SymbolKind::Function`]; ignored elsewhere.
    pub is_inline: bool,
    /// C17 §6.7.4 `_Noreturn` function specifier.
    pub is_noreturn: bool,
    /// GNU `__attribute__((noreturn))` — merged with `is_noreturn` for
    /// the purposes of control-flow analysis but kept separate so
    /// diagnostics can attribute the source correctly.
    pub has_noreturn_attr: bool,
    /// `true` if the address of this declaration may have escaped the
    /// function body — i.e. somewhere in scope `&local` or array-to-
    /// pointer decay was applied to it.  Only meaningful on locals
    /// ([`SymbolKind::Variable`] with [`StorageClass::None`] and
    /// [`Linkage::None`]); for globals and parameters the flag stays at
    /// its `false` default and Phase 5 IR lowering treats those as
    /// unconditionally memory-resident anyway.
    pub address_taken: bool,
}

// =========================================================================
// Tag entries
// =========================================================================

/// An entry in the tag namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagEntry {
    /// A `struct` tag referencing a [`StructTypeId`].
    Struct(StructTypeId),
    /// A `union` tag referencing a [`UnionTypeId`].
    Union(UnionTypeId),
    /// An `enum` tag referencing an [`EnumTypeId`].
    Enum(EnumTypeId),
}

impl TagEntry {
    fn kind_str(&self) -> &'static str {
        match self {
            TagEntry::Struct(_) => "struct",
            TagEntry::Union(_) => "union",
            TagEntry::Enum(_) => "enum",
        }
    }

    /// `true` if `self` and `other` name the same underlying type.
    fn same_id(&self, other: &TagEntry) -> bool {
        match (self, other) {
            (TagEntry::Struct(a), TagEntry::Struct(b)) => a == b,
            (TagEntry::Union(a), TagEntry::Union(b)) => a == b,
            (TagEntry::Enum(a), TagEntry::Enum(b)) => a == b,
            _ => false,
        }
    }
}

// =========================================================================
// Scope
// =========================================================================

/// A single lexical scope.
///
/// The ordinary namespace maps names to dense symbol ids; the tag
/// namespace maps names to dense tag ids.  The two are independent —
/// `struct foo { int x; }; int foo;` is legal C.
#[derive(Clone, Debug)]
pub struct Scope {
    /// Kind of scope (file, function, block, prototype).
    pub kind: ScopeKind,
    /// Ordinary namespace: variable / function / typedef / enumerator
    /// / parameter names.
    pub symbols: HashMap<String, SymbolId>,
    /// Tag namespace: `struct`, `union`, and `enum` tags.
    pub tags: HashMap<String, TagId>,
}

impl Scope {
    fn new(kind: ScopeKind) -> Self {
        Self {
            kind,
            symbols: HashMap::new(),
            tags: HashMap::new(),
        }
    }
}

// =========================================================================
// SymbolTable
// =========================================================================

/// The stack of lexical scopes for a translation unit.
///
/// A fresh table starts with one pushed [`ScopeKind::File`] scope so
/// that callers can insert file-scope declarations without a leading
/// `push_scope`.  Lookups walk the stack from innermost to outermost.
#[derive(Debug)]
pub struct SymbolTable {
    scopes: Vec<Scope>,
    all_symbols: Vec<Symbol>,
    all_tags: Vec<TagEntry>,
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolTable {
    /// Build a new table pre-populated with a single file scope.
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope::new(ScopeKind::File)],
            all_symbols: Vec::new(),
            all_tags: Vec::new(),
        }
    }

    // ---- scope management ----

    /// Push a new inner scope.
    pub fn push_scope(&mut self, kind: ScopeKind) {
        self.scopes.push(Scope::new(kind));
    }

    /// Pop the innermost scope.  Panics if only the file scope remains —
    /// the file scope is the root and must stay.
    pub fn pop_scope(&mut self) {
        assert!(
            self.scopes.len() > 1,
            "pop_scope underflow: cannot pop the file scope"
        );
        self.scopes.pop();
    }

    /// The innermost scope (always present).
    pub fn current_scope(&self) -> &Scope {
        // SAFETY: `SymbolTable::new` always pushes a file scope, and
        // `pop_scope` asserts `scopes.len() > 1`, so `scopes` is never
        // empty for any constructable value of this type.
        self.scopes
            .last()
            .expect("symbol table always has at least a file scope")
    }

    /// Kind of the innermost scope — shorthand for
    /// `self.current_scope().kind`.
    pub fn current_scope_kind(&self) -> ScopeKind {
        self.current_scope().kind
    }

    /// Number of currently-open scopes, counting the file scope.
    /// Primarily useful for tests.
    pub fn scope_depth(&self) -> usize {
        self.scopes.len()
    }

    // ---- ordinary namespace ----

    /// Declare `symbol` in the current scope.
    ///
    /// Returns the dense [`SymbolId`] on success.  A same-name entry
    /// already in the current scope causes one of three things:
    ///
    /// * compatible extern redeclaration → merge types and return the
    ///   original id;
    /// * compatible tentative object or extern at file scope → merge
    ///   and return the original id;
    /// * otherwise → emit an error to `ctx` and return `None`.
    pub fn declare(&mut self, symbol: Symbol, ctx: &mut SemaContext) -> Option<SymbolId> {
        // SAFETY: `scopes` is non-empty by the invariant established in
        // `SymbolTable::new` and preserved by `pop_scope`.
        let top = self.scopes.last().expect("file scope always present");
        if let Some(&existing_id) = top.symbols.get(&symbol.name) {
            return self.try_merge_redeclaration(existing_id, symbol, ctx);
        }

        let id = self.intern_symbol(symbol);
        // SAFETY: same invariant — `scopes` is non-empty.
        let top = self.scopes.last_mut().expect("file scope always present");
        let name = self.all_symbols[id as usize].name.clone();
        top.symbols.insert(name, id);
        Some(id)
    }

    /// Look up `name` in the ordinary namespace, walking outward from
    /// the innermost scope.
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        for scope in self.scopes.iter().rev() {
            if let Some(&id) = scope.symbols.get(name) {
                return Some(&self.all_symbols[id as usize]);
            }
        }
        None
    }

    /// Look up `name` only in the innermost scope.
    pub fn lookup_in_current_scope(&self, name: &str) -> Option<&Symbol> {
        let top = self.scopes.last()?;
        let id = *top.symbols.get(name)?;
        Some(&self.all_symbols[id as usize])
    }

    /// Borrow a symbol by id.
    pub fn symbol(&self, id: SymbolId) -> &Symbol {
        &self.all_symbols[id as usize]
    }

    /// Number of symbols interned so far (used by tests).
    pub fn symbol_count(&self) -> usize {
        self.all_symbols.len()
    }

    /// Every symbol the table has interned, in insertion order.
    ///
    /// Used by end-of-translation-unit passes that need to walk every
    /// ordinary-namespace declaration regardless of which scope it
    /// belongs to (e.g. promoting tentative definitions — C17 §6.9.2).
    pub fn all_symbols(&self) -> &[Symbol] {
        &self.all_symbols
    }

    /// Flip `is_defined` to `true` for the given symbol.
    ///
    /// Used to promote a file-scope tentative definition to a real
    /// definition at end of translation unit (C17 §6.9.2).  Caller is
    /// responsible for verifying the symbol satisfies the tentative
    /// definition rules before calling.
    pub fn mark_defined(&mut self, id: SymbolId) {
        self.all_symbols[id as usize].is_defined = true;
    }

    /// Set `address_taken` to `true` for the given symbol.  Idempotent.
    ///
    /// Called by [`crate::address_taken::analyze_address_taken`] each
    /// time it discovers a `&local` or array-decay site referencing
    /// this symbol.  Repeated calls collapse — the flag is monotone.
    pub fn mark_address_taken(&mut self, id: SymbolId) {
        self.all_symbols[id as usize].address_taken = true;
    }

    // ---- tag namespace ----

    /// Declare a tag in the current scope.
    ///
    /// A matching tag already in scope with the same kind is considered
    /// a re-reference, and the existing id is returned.  A matching
    /// tag with a *different* kind (e.g. `struct foo` vs `union foo`)
    /// is an error.
    pub fn declare_tag(
        &mut self,
        name: &str,
        entry: TagEntry,
        span: Span,
        ctx: &mut SemaContext,
    ) -> Option<TagId> {
        // SAFETY: `scopes` is non-empty by the invariant established in
        // `SymbolTable::new` and preserved by `pop_scope`.
        if let Some(&existing_id) = self
            .scopes
            .last()
            .expect("file scope always present")
            .tags
            .get(name)
        {
            let existing = &self.all_tags[existing_id as usize];
            if std::mem::discriminant(existing) == std::mem::discriminant(&entry) {
                // Same kind, same tag — accept.  If the IDs differ the
                // caller is asking us to unify two different StructTypeIds
                // under one name, which is a bug upstream.
                if !existing.same_id(&entry) {
                    ctx.emit(tag_redeclared_different_id(name, existing, &entry, span));
                    return None;
                }
                return Some(existing_id);
            }
            ctx.emit(tag_kind_mismatch(name, existing, &entry, span));
            return None;
        }

        let id = self.intern_tag(entry);
        // SAFETY: same invariant — `scopes` is non-empty.
        let top = self.scopes.last_mut().expect("file scope always present");
        top.tags.insert(name.to_string(), id);
        Some(id)
    }

    /// Look up `name` in the tag namespace, walking outward from the
    /// innermost scope.
    pub fn lookup_tag(&self, name: &str) -> Option<(TagId, &TagEntry)> {
        for scope in self.scopes.iter().rev() {
            if let Some(&id) = scope.tags.get(name) {
                return Some((id, &self.all_tags[id as usize]));
            }
        }
        None
    }

    /// Look up `name` only in the innermost scope's tag namespace.
    pub fn lookup_tag_in_current_scope(&self, name: &str) -> Option<(TagId, &TagEntry)> {
        let top = self.scopes.last()?;
        let id = *top.tags.get(name)?;
        Some((id, &self.all_tags[id as usize]))
    }

    /// Borrow a tag entry by id.
    pub fn tag(&self, id: TagId) -> &TagEntry {
        &self.all_tags[id as usize]
    }

    /// Number of interned tags — for tests.
    pub fn tag_count(&self) -> usize {
        self.all_tags.len()
    }

    // ---- internals ----

    fn intern_symbol(&mut self, mut symbol: Symbol) -> SymbolId {
        // SAFETY: a translation unit with >4 billion symbols is beyond
        // any real-world C program; this overflow is unreachable in
        // practice.
        let id =
            u32::try_from(self.all_symbols.len()).expect("more than 4 billion symbols — give up");
        symbol.id = id;
        self.all_symbols.push(symbol);
        id
    }

    fn intern_tag(&mut self, entry: TagEntry) -> TagId {
        // SAFETY: a translation unit with >4 billion tags is beyond any
        // real-world C program; this overflow is unreachable in practice.
        let id = u32::try_from(self.all_tags.len()).expect("more than 4 billion tags — give up");
        self.all_tags.push(entry);
        id
    }

    fn try_merge_redeclaration(
        &mut self,
        existing_id: SymbolId,
        mut new_sym: Symbol,
        ctx: &mut SemaContext,
    ) -> Option<SymbolId> {
        let existing = &self.all_symbols[existing_id as usize];

        // Differing symbol kinds are always a hard error (function vs
        // variable, typedef vs variable, etc.).
        if std::mem::discriminant(&existing.kind) != std::mem::discriminant(&new_sym.kind) {
            ctx.emit(kind_redeclaration_mismatch(existing, &new_sym));
            return None;
        }

        // Typedefs can only be redeclared if the types are compatible
        // (C17 §6.7 allows redundant typedef redeclarations with the
        // same type, but we keep the rule permissive: identical shape
        // is fine).
        if matches!(existing.kind, SymbolKind::Typedef) {
            if existing.ty == new_sym.ty {
                return Some(existing_id);
            }
            ctx.emit(incompatible_redeclaration(existing, &new_sym));
            return None;
        }

        // Object or function redeclaration: types must be compatible.
        if !are_compatible(&existing.ty, &new_sym.ty, &ctx.type_ctx) {
            ctx.emit(incompatible_redeclaration(existing, &new_sym));
            return None;
        }

        // Definition rules: at most one non-tentative definition.
        if existing.is_defined && new_sym.is_defined {
            ctx.emit(duplicate_definition(existing, &new_sym));
            return None;
        }

        // Merge: compute composite type, promote to "defined" if either
        // side defined, keep the stronger storage class (static wins
        // over extern on first-seen static; extern does not weaken
        // previous linkage).
        let merged_ty = composite_type(&existing.ty, &new_sym.ty, &ctx.type_ctx);
        let merged_defined = existing.is_defined || new_sym.is_defined;
        let merged_storage = merge_storage(existing.storage, new_sym.storage);
        let merged_linkage = merge_linkage(existing.linkage, new_sym.linkage);

        let slot = &mut self.all_symbols[existing_id as usize];
        slot.ty = merged_ty;
        slot.is_defined = merged_defined;
        slot.storage = merged_storage;
        slot.linkage = merged_linkage;
        // The new_sym span becomes the "most recent" span — not strictly
        // necessary but convenient for diagnostics.
        new_sym.span = slot.span;
        Some(existing_id)
    }
}

// =========================================================================
// Diagnostic helpers
// =========================================================================

fn kind_redeclaration_mismatch(existing: &Symbol, new_sym: &Symbol) -> Diagnostic {
    Diagnostic::error(format!(
        "redeclaration of '{}' with a different kind",
        new_sym.name
    ))
    .span(new_sym.span)
    .label_at(existing.span, "previously declared here")
}

fn incompatible_redeclaration(existing: &Symbol, new_sym: &Symbol) -> Diagnostic {
    Diagnostic::error(format!(
        "redeclaration of '{}' with incompatible type",
        new_sym.name
    ))
    .span(new_sym.span)
    .label_at(existing.span, "previously declared here")
}

fn duplicate_definition(existing: &Symbol, new_sym: &Symbol) -> Diagnostic {
    Diagnostic::error(format!("redefinition of '{}'", new_sym.name))
        .span(new_sym.span)
        .label_at(existing.span, "first defined here")
}

fn tag_kind_mismatch(name: &str, old: &TagEntry, new: &TagEntry, span: Span) -> Diagnostic {
    Diagnostic::error(format!(
        "'{}' defined as a '{}' but previously declared as a '{}'",
        name,
        new.kind_str(),
        old.kind_str()
    ))
    .span(span)
}

fn tag_redeclared_different_id(
    name: &str,
    _old: &TagEntry,
    _new: &TagEntry,
    span: Span,
) -> Diagnostic {
    Diagnostic::error(format!(
        "tag '{name}' already declared in this scope with a different identity"
    ))
    .span(span)
}

// =========================================================================
// Merge helpers
// =========================================================================

fn merge_storage(a: StorageClass, b: StorageClass) -> StorageClass {
    // Keep the non-`None` side; prefer the stronger qualifier if both
    // are set (`static` is stronger than `extern`).  This matches the
    // informal rule that `static int x;` followed by `extern int x;`
    // stays `static`.
    match (a, b) {
        (StorageClass::None, other) | (other, StorageClass::None) => other,
        (StorageClass::Static, _) | (_, StorageClass::Static) => StorageClass::Static,
        _ => a,
    }
}

fn merge_linkage(a: Linkage, b: Linkage) -> Linkage {
    match (a, b) {
        (Linkage::None, other) | (other, Linkage::None) => other,
        (Linkage::Internal, _) | (_, Linkage::Internal) => Linkage::Internal,
        _ => Linkage::External,
    }
}
