use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, ClearType},
};
use std::io::{self, Write};

pub enum Choice {
    Select(String),
    /// < or Esc: go back to parent menu
    Back,
    /// q: quit at the top menu, go back elsewhere
    Quit,
}

/// Shows a navigable list. Returns the selected string, or a back/quit signal.
///
/// Keys: ↑/k up, ↓/j down, Enter/> select, </Esc back, q quit-or-back.
pub fn select(prompt: &str, options: &[impl AsRef<str>]) -> io::Result<Choice> {
    let opts: Vec<&str> = options.iter().map(|o| o.as_ref()).collect();
    let n = opts.len();
    if n == 0 {
        return Ok(Choice::Back);
    }

    let mut sel = 0usize;
    let mut out = io::stdout();

    terminal::enable_raw_mode()?;
    execute!(out, cursor::Hide)?;

    render(&mut out, prompt, &opts, sel)?;

    let choice = loop {
        match event::read()? {
            Event::Key(k) => {
                if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
                    let _ = terminal::disable_raw_mode();
                    let _ = execute!(out, cursor::Show);
                    std::process::exit(130);
                }
                match k.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        sel = if sel == 0 { n - 1 } else { sel - 1 };
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        sel = if sel + 1 < n { sel + 1 } else { 0 };
                    }
                    KeyCode::Enter | KeyCode::Char('>') => {
                        break Choice::Select(opts[sel].to_string());
                    }
                    KeyCode::Esc | KeyCode::Char('<') => break Choice::Back,
                    KeyCode::Char('q') => break Choice::Quit,
                    _ => continue,
                }
            }
            _ => continue,
        }
        erase(&mut out, n + 1)?;
        render(&mut out, prompt, &opts, sel)?;
    };

    erase(&mut out, n + 1)?;
    terminal::disable_raw_mode()?;
    execute!(out, cursor::Show)?;

    if let Choice::Select(ref s) = choice {
        println!("{s}");
    }

    Ok(choice)
}

fn render(out: &mut impl Write, prompt: &str, opts: &[&str], sel: usize) -> io::Result<()> {
    queue!(out, terminal::Clear(ClearType::CurrentLine), Print(format!("{prompt}\r\n")))?;
    for (i, opt) in opts.iter().enumerate() {
        queue!(out, terminal::Clear(ClearType::CurrentLine))?;
        if i == sel {
            queue!(
                out,
                SetForegroundColor(Color::Cyan),
                Print(format!("❯ {opt}\r\n")),
                ResetColor,
            )?;
        } else {
            queue!(out, Print(format!("  {opt}\r\n")))?;
        }
    }
    out.flush()
}

fn erase(out: &mut impl Write, lines: usize) -> io::Result<()> {
    if lines == 0 {
        return Ok(());
    }
    queue!(out, cursor::MoveUp(lines as u16))?;
    for _ in 0..lines {
        queue!(out, terminal::Clear(ClearType::CurrentLine), Print("\r\n"))?;
    }
    queue!(out, cursor::MoveUp(lines as u16))?;
    out.flush()
}
