use super::super::source::SourceLocation;
use crate::kernel::Span;

pub fn source_location(path: &str, base_line: usize, span: &Span) -> SourceLocation {
    SourceLocation {
        path: path.into(),
        line: base_line + span.start.line,
        column: span.start.column,
        end_line: base_line + span.end.line,
        end_column: span.end.column,
    }
}
