use super::*;

impl Compiler {
    pub(super) fn compile_await(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if children.len() != 2 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "co/await expects one promise",
                Some(span.start),
            ));
        }
        if !self.ctx().suspend_allowed {
            return Err(CompileError::new(
                CompileErrorKind::InvalidEffect,
                "co/await requires ^:async or co/create",
                Some(span.start),
            ));
        }
        let promise = &children[1];
        self.compile_form(promise.form, promise.span, promise.children, false)?;
        if self.ctx().fallthrough {
            self.emit(Instruction::Await, Some(span.start));
        }
        Ok(())
    }

    pub(super) fn compile_yield(
        &mut self,
        children: &[Child<'_>],
        span: &Span,
    ) -> Result<(), CompileError> {
        if children.len() != 2 {
            return Err(CompileError::new(
                CompileErrorKind::Arity,
                "co/yield expects one value",
                Some(span.start),
            ));
        }
        if !self.ctx().suspend_allowed {
            return Err(CompileError::new(
                CompileErrorKind::InvalidEffect,
                "co/yield requires a coroutine function",
                Some(span.start),
            ));
        }
        let value = &children[1];
        self.compile_form(value.form, value.span, value.children, false)?;
        if self.ctx().fallthrough {
            self.emit(Instruction::Yield, Some(span.start));
        }
        Ok(())
    }
}
