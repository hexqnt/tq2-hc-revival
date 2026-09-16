use std::{
    error::Error,
    io::{self, Write},
    path::{Path, PathBuf},
};

use clap::{Parser, Subcommand};
use colored::Colorize;
use comfy_table::{Attribute, Cell, Color, ContentArrangement, Table, presets::NOTHING};
use time::{OffsetDateTime, UtcOffset, macros::format_description};

mod paths;
mod save;
mod storage;

#[derive(Subcommand)]
enum Command {
    /// List all characters and their save statistics.
    List,

    /// Revive a character by name, or all dead characters with --all.
    Revive {
        #[arg(long, conflicts_with = "character")]
        all: bool,

        #[arg(required_unless_present = "all")]
        character: Option<String>,
    },
}

#[derive(Parser)]
#[command(version, about = "Revive dead Titan Quest 2 Hardcore characters")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to the TQ2 Saved/SaveGames directory.
    #[arg(long, global = true)]
    save_dir: Option<PathBuf>,
}

fn run() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    println!(
        "{} {}\n",
        "Titan Quest 2 HC Revival".cyan().bold(),
        concat!("v", env!("CARGO_PKG_VERSION")).bright_black()
    );
    let directory = match cli.save_dir {
        Some(path) => path,
        None => select_save_dir(cli.command.is_none())?,
    };
    let characters = storage::characters(&directory)?;

    match cli.command {
        Some(Command::List) => {
            if characters.is_empty() {
                println!("No characters found.");
            } else {
                print_characters(&characters);
            }
        }
        Some(Command::Revive { character, all }) => {
            if all {
                revive_many(&directory, characters.iter().filter(|entry| entry.is_dead))?;
            } else {
                let character = character.ok_or("Specify a character name or --all")?;
                let mut matches = characters
                    .iter()
                    .filter(|entry| entry.is_dead && entry.name == character);
                let selected = matches.next().ok_or("Dead Hardcore character not found")?;
                if matches.next().is_some() {
                    return Err("Multiple dead Hardcore characters have that name; select by number in interactive mode".into());
                }
                revive_many(&directory, std::iter::once(selected))?;
            }
        }
        None => {
            let dead: Vec<_> = characters.iter().filter(|entry| entry.is_dead).collect();
            if dead.is_empty() {
                println!(
                    "No dead Hardcore characters found in {}.",
                    directory.display()
                );
                return Ok(());
            }
            println!("{}", "Dead Hardcore characters:".cyan().bold());
            for (index, character) in dead.iter().enumerate() {
                println!(
                    "  {}. {}",
                    (index + 1).to_string().yellow(),
                    character.name.green()
                );
            }
            let input = prompt("Select a character number to revive (q to quit): ")?;
            if input.eq_ignore_ascii_case("q") {
                return Ok(());
            }
            let choice: usize = input.parse()?;
            let selected = choice
                .checked_sub(1)
                .and_then(|index| dead.get(index))
                .ok_or("Invalid character number")?;
            revive_many(&directory, std::iter::once(*selected))?;
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{} {error}", "Error:".red().bold());
        std::process::exit(1);
    }
}

fn prompt(message: &str) -> io::Result<String> {
    print!("{}", message.yellow());
    io::stdout().flush()?;
    let mut input = String::new();
    if io::stdin().read_line(&mut input)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "No input received",
        ));
    }
    Ok(input.trim().to_owned())
}

fn revive_many<'a>(
    directory: &Path,
    characters: impl IntoIterator<Item = &'a storage::Character>,
) -> Result<(), Box<dyn Error>> {
    let mut characters = characters.into_iter().peekable();
    if characters.peek().is_none() {
        println!(
            "No dead Hardcore characters found in {}.",
            directory.display()
        );
        return Ok(());
    }
    eprintln!(
        "{}",
        "Close Titan Quest 2 and disable Steam Cloud Sync before reviving.".yellow()
    );
    for character in characters {
        let backup = storage::revive(directory, character)?;
        println!(
            "Revived {}. Original files are in {}.",
            character.name.green(),
            backup.display()
        );
    }
    Ok(())
}

fn format_duration(seconds: u64) -> String {
    format!(
        "{}h {:02}m {:02}s",
        seconds / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

fn select_save_dir(interactive: bool) -> Result<PathBuf, Box<dyn Error>> {
    let mut directories = paths::discover_save_dirs();
    if directories.len() == 1 {
        return Ok(directories.remove(0));
    }
    if directories.is_empty() {
        return if interactive {
            Ok(PathBuf::from(prompt("SaveGames directory: ")?))
        } else {
            Err("SaveGames directory not found; pass --save-dir PATH".into())
        };
    }
    if !interactive {
        let options = directories
            .iter()
            .map(|path| format!("  {}", path.display()))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!(
            "Multiple SaveGames directories found; pass --save-dir PATH:\n{options}"
        )
        .into());
    }
    println!("{}", "SaveGames directories:".cyan().bold());
    for (index, path) in directories.iter().enumerate() {
        println!("  {}. {}", index + 1, path.display());
    }
    let choice: usize = prompt("Select a SaveGames directory: ")?.parse()?;
    directories
        .into_iter()
        .nth(choice.checked_sub(1).ok_or("Invalid directory number")?)
        .ok_or_else(|| "Invalid directory number".into())
}
fn print_characters(characters: &[storage::Character]) {
    let mut table = Table::new();
    table.load_style(NOTHING);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.style_text_only();
    if std::env::var_os("NO_COLOR").is_some() {
        table.force_no_tty();
    }
    table.set_header(
        [
            "Name",
            "Status",
            "Difficulty",
            "Level",
            "Last save (local)",
            "Playtime",
            "Deaths",
            "At last death",
        ]
        .map(|title| {
            Cell::new(title)
                .fg(Color::DarkCyan)
                .add_attribute(Attribute::Bold)
        }),
    );
    for column in table.column_iter_mut() {
        column.set_padding((0, 1));
    }
    for character in characters {
        let details = &character.details;
        let last_save = details
            .last_save_ticks
            .and_then(|ticks| {
                const UNIX_EPOCH_TICKS: i128 = 621_355_968_000_000_000;
                OffsetDateTime::from_unix_timestamp_nanos(
                    (i128::from(ticks) - UNIX_EPOCH_TICKS) * 100,
                )
                .ok()
            })
            .and_then(|utc| {
                let offset = UtcOffset::local_offset_at(utc).ok()?;
                utc.to_offset(offset)
                    .format(&format_description!("[year]-[month]-[day] [hour]:[minute]"))
                    .ok()
            })
            .unwrap_or_else(|| "—".to_owned());
        let level = details
            .level
            .map(|level| format!("{level:.0}"))
            .unwrap_or_else(|| "—".to_owned());
        let deaths = character
            .deaths
            .count
            .map_or_else(|| "—".to_owned(), |count| count.to_string());
        let last_death = character
            .deaths
            .play_seconds_at_last_death
            .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
            .map(|seconds| format_duration(seconds as u64))
            .unwrap_or_else(|| "—".to_owned());
        let status = if character.is_dead {
            Cell::new("Dead").fg(Color::Red)
        } else {
            Cell::new("Alive").fg(Color::Green)
        };
        table.add_row([
            Cell::new(&character.name).add_attribute(Attribute::Bold),
            status,
            Cell::new(details.difficulty.as_deref().unwrap_or("—")),
            Cell::new_owned(level),
            Cell::new_owned(last_save),
            Cell::new_owned(
                details
                    .play_seconds
                    .map_or_else(|| "—".to_owned(), format_duration),
            ),
            Cell::new_owned(deaths),
            Cell::new_owned(last_death),
        ]);
    }
    println!("{table}");
}

