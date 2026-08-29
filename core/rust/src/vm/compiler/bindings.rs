//! Compilation of immutable and mutable global bindings.
//!
//! This child owns `def`, `set!`, and `var`; namespace declaration,
//! callable publication, structures, fields, and functions remain in
//! `globals.rs`. The split is structural only and preserves the emitted
//! instruction sequences.

use crate::core::binding_symbol;
use crate::kernel::{Form, Span};
use crate::vm::error::{CompileError, CompileErrorKind};
use crate::vm::opcode::Instruction;

use super::{Child, Compiler};

impl Compiler {
    /// `(def name init)`: interns the value in the current namespace and
    /// evaluates to the interned Var. The name
    /// becomes visible only after the initializer compiles, so an
    /// initializer cannot self-reference — matching the evaluator's
    /// "unbound symbol" for `(def x x)` on a fresh name.
    pub(super) fn compile_def(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if children.len() != 3 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "def expects a name and value",
                Some(span.start),
            ));
        }
        let (name, metadata) = binding_symbol(children[1].form, "def name")
            .map_err(|message| unsupported(message, children[1].span.start))?;
        self.require_owned_global(&name, children[1].span)?;
        let metadata = self.var_metadata(metadata);
        let initializer = &children[2];
        self.compile_form(
            initializer.form,
            initializer.span,
            initializer.children,
            false,
        )?;
        if !self.ctx().fallthrough {
            return Ok(());
        }
        self.declare_program_global(&name);
        let name_index = self.name_constant(&name, children[1].span)?;
        self.emit(
            Instruction::DefGlobal {
                name: name_index,
                metadata,
            },
            Some(span.start),
        );
        Ok(())
    }

    /// `(set! name value)` resets a global var. `(set! (field receiver
    /// :name) value)` evaluates receiver and replacement once, in that order,
    /// then mutates a declared field and evaluates to the replacement.
    pub(super) fn compile_set(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if children.len() != 3 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "set! expects a place and value",
                Some(span.start),
            ));
        }
        if let Form::List(place) = children[1].form {
            if matches!(place.first(), Some(Form::Symbol(operation)) if operation == "field") {
                let place_children =
                    self.list_children(place, children[1].span, children[1].children);
                if place_children.len() != 3 {
                    return Err(CompileError::new(
                        CompileErrorKind::Arity,
                        "set! field place expects a receiver and field",
                        Some(children[1].span.start),
                    ));
                }
                let field = match place_children[2].form {
                    Form::Keyword(field) | Form::Symbol(field) if !field.contains('/') => field,
                    _ => {
                        return Err(unsupported(
                            "set! field place expects an unqualified literal field",
                            place_children[2].span.start,
                        ))
                    }
                };
                self.compile_form(
                    place_children[1].form,
                    place_children[1].span,
                    place_children[1].children,
                    false,
                )?;
                if !self.ctx().fallthrough {
                    return Ok(());
                }
                self.compile_form(
                    children[2].form,
                    children[2].span,
                    children[2].children,
                    false,
                )?;
                if !self.ctx().fallthrough {
                    return Ok(());
                }
                let index = self.name_constant(field, place_children[2].span)?;
                self.emit(Instruction::MutableFieldSet(index), Some(span.start));
                return Ok(());
            }
        }
        let Form::Symbol(name) = children[1].form else {
            return Err(unsupported(
                "set! expects a name symbol or field place",
                children[1].span.start,
            ));
        };
        if self.ctx().scopes.resolve(name).is_some() {
            return Err(unsupported(
                format!("set! targets a global var: {name} is a lexical binding"),
                children[1].span.start,
            ));
        }
        if !self.visible_global(name) {
            return Err(CompileError::new(
                CompileErrorKind::UnboundSymbol,
                format!("unbound var: {name}"),
                Some(children[1].span.start),
            ));
        }
        self.require_owned_global(name, children[1].span)?;
        let value = &children[2];
        self.compile_form(value.form, value.span, value.children, false)?;
        if !self.ctx().fallthrough {
            return Ok(());
        }
        let index = self.name_constant(name, children[1].span)?;
        self.emit(Instruction::SetGlobal(index), Some(span.start));
        Ok(())
    }

    /// `(var name)` (also the `#'name` reader form): pushes the var
    /// itself. Lexical bindings are not vars; an invisible name is
    /// "unbound var".
    pub(super) fn compile_var(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if children.len() != 2 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "var expects a name symbol",
                Some(span.start),
            ));
        }
        let Form::Symbol(name) = children[1].form else {
            return Err(unsupported(
                "var expects a name symbol",
                children[1].span.start,
            ));
        };
        if !self.visible_global(name) {
            return Err(CompileError::new(
                CompileErrorKind::UnboundSymbol,
                format!("unbound var: {name}"),
                Some(children[1].span.start),
            ));
        }
        let index = self.name_constant(name, children[1].span)?;
        self.emit(Instruction::VarGlobal(index), Some(span.start));
        Ok(())
    }
}

fn unsupported(message: impl Into<String>, position: crate::kernel::Position) -> CompileError {
    CompileError::new(
        CompileErrorKind::UnsupportedForm,
        message.into(),
        Some(position),
    )
}
