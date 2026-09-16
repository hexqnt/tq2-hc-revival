# Titan Quest 2 HC Revival

![Poster](poster.gif)

[🇺🇸 English](./README.md) · [🇷🇺 Русский](./README.ru.md)

A command-line tool that lists saved characters and revives dead Hardcore characters by removing `m_IsPermanentlyDead` from their `*_Header.sav` files. It finds the property by name, without relying on a fixed offset.

## Before you start

Revival usually works without these precautions, but for reliability, close Titan Quest 2 and disable Steam Cloud Sync for the game so the revived save is not overwritten.

## Install and use

Download the archive for your platform from [GitHub Releases](https://github.com/hexqnt/tq2-hc-revival/releases), unpack it, and run the executable. To install from source instead, run `cargo install --path .` in the source directory. With the installed command (or the path to the unpacked executable), run:

```sh
tq2-hc-revival # Interactively choose a character to revive
tq2-hc-revival list # List all characters
tq2-hc-revival revive Lina # Revive the character named "Lina"
tq2-hc-revival revive --all # Revive all dead characters
```

The tool finds SaveGames automatically. Without a command, it lists dead characters and lets you select one by number. `list` shows status, difficulty, level, last save, playtime, and available death statistics; missing fields appear as `—`. Only dead Hardcore characters can be revived. `revive --all` revives all of them in the selected directory. Quote names containing spaces, for example `revive "My Hero"`.

If automatic detection fails, or a command finds multiple save directories, specify the directory explicitly (the option works with any command):

```sh
tq2-hc-revival --save-dir "/path/to/SaveGames" revive Hexen
```

Interactive mode asks for a path if none is found, or lets you select a directory if several are found. Detection checks `%LOCALAPPDATA%\TQ2\Saved\SaveGames` on Windows, Steam libraries on all platforms, Proton prefixes on Linux, and the usual TQ2, CrossOver, and Whisky locations on macOS.

Before editing a character, the tool moves its original header, SaveGames `.bak` files, and `Saving.sav` into a timestamped backup directory next to SaveGames. `revive --all` creates a separate backup for each character. Other character files and death statistics remain unchanged.
