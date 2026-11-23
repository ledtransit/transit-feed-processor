use std::fmt::{Display, Write as _};

use anyhow::{Context, Result};
use chrono::NaiveDate;
use sqlx::PgConnection;

/// One tab-separated row for Postgres `COPY ... FROM STDIN` text format:
/// `\t` between fields, `\N` for NULL, backslash escapes for control chars.
pub struct Row<'a> {
    buf: &'a mut String,
    first: bool,
}

impl Row<'_> {
    fn sep(&mut self) {
        if self.first {
            self.first = false;
        } else {
            self.buf.push('\t');
        }
    }

    pub fn text(&mut self, s: &str) {
        self.sep();
        if s.bytes()
            .any(|b| matches!(b, b'\\' | b'\t' | b'\n' | b'\r'))
        {
            for c in s.chars() {
                match c {
                    '\\' => self.buf.push_str("\\\\"),
                    '\t' => self.buf.push_str("\\t"),
                    '\n' => self.buf.push_str("\\n"),
                    '\r' => self.buf.push_str("\\r"),
                    _ => self.buf.push(c),
                }
            }
        } else {
            self.buf.push_str(s);
        }
    }

    pub fn opt_text(&mut self, s: Option<&str>) {
        match s {
            Some(s) => self.text(s),
            None => self.null(),
        }
    }

    pub fn num(&mut self, v: impl Display) {
        self.sep();
        let _ = write!(self.buf, "{v}");
    }

    pub fn opt_num<T: Display>(&mut self, v: Option<T>) {
        match v {
            Some(v) => self.num(v),
            None => self.null(),
        }
    }

    pub fn bool(&mut self, v: bool) {
        self.sep();
        self.buf.push(if v { 't' } else { 'f' });
    }

    pub fn date(&mut self, d: NaiveDate) {
        self.sep();
        let _ = write!(self.buf, "{d}");
    }

    pub fn null(&mut self) {
        self.sep();
        self.buf.push_str("\\N");
    }
}

/// Render `rows` to a `COPY` text body (one `\n`-terminated line per row).
pub fn body<T>(rows: &[T], mut render: impl FnMut(&mut Row<'_>, &T)) -> String {
    let mut buf = String::with_capacity(rows.len().saturating_mul(64));
    for row in rows {
        line(&mut buf, |w| render(w, row));
    }
    buf
}

/// Append one `\n`-terminated row to a `COPY` text body.
pub fn line(buf: &mut String, render: impl FnOnce(&mut Row<'_>)) {
    render(&mut Row { buf, first: true });
    buf.push('\n');
}

/// Stream a pre-rendered `COPY <stmt> FROM STDIN` body to Postgres.
pub async fn send(conn: &mut PgConnection, stmt: &str, body: &str) -> Result<()> {
    if body.is_empty() {
        return Ok(());
    }
    let mut sink = conn
        .copy_in_raw(stmt)
        .await
        .with_context(|| format!("begin {stmt}"))?;
    for frame in body.as_bytes().chunks(4 << 20) {
        sink.send(frame).await.context("copy send")?;
    }
    sink.finish().await.context("copy finish")?;
    Ok(())
}
