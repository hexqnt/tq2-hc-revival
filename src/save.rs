use std::{io, ops::Range};

use time::{OffsetDateTime, macros::format_description};

const NAME: &[u8] = b"\x14\0\0\0m_IsPermanentlyDead\0";
const CHARACTER_NAME: &[u8] = b"\x10\0\0\0m_CharacterName\0";

pub struct DeathStats {
    pub count: Option<u64>,

    pub play_seconds_at_last_death: Option<f64>,
}

pub struct HeaderDetails {
    pub level: Option<f32>,

    pub difficulty: Option<String>,

    pub play_seconds: Option<u64>,

    pub last_save_ticks: Option<u64>,
}

pub struct PropertyInfo {
    pub name: String,
    pub kind: String,
    pub value: String,
}

/// Finds `m_` GVAS properties without assuming a fixed save layout.
pub fn inspect_properties(bytes: &[u8]) -> io::Result<Vec<PropertyInfo>> {
    if !bytes.starts_with(b"GVAS") {
        return Err(invalid("Not an Unreal GVAS save"));
    }
    let mut properties = Vec::new();
    for (offset, window) in bytes.windows(2).enumerate() {
        if window != b"m_" || offset < 4 {
            continue;
        }
        let start = offset - 4;
        let Ok((name, next)) = fstring(bytes, start) else {
            continue;
        };
        if !name.starts_with(b"m_") || next != offset + name.len() + 1 {
            continue;
        }
        let Ok((kind, value_offset)) = fstring(bytes, next) else {
            continue;
        };
        if !kind.ends_with(b"Property") {
            continue;
        }
        properties.push(PropertyInfo {
            name: String::from_utf8_lossy(name).into_owned(),
            kind: String::from_utf8_lossy(kind).into_owned(),
            value: inspected_value(bytes, kind, value_offset).unwrap_or_else(|| "—".to_owned()),
        });
    }
    Ok(properties)
}

fn inspected_value(bytes: &[u8], kind: &[u8], offset: usize) -> Option<String> {
    if kind == b"BoolProperty" {
        let metadata = bytes.get(offset..offset + 9)?;
        if metadata[..8] != [0; 8] {
            return None;
        }
        return match metadata[8] {
            0 => Some("false".to_owned()),
            1 | 0x10 => Some("true".to_owned()),
            _ => None,
        };
    }
    if matches!(
        kind,
        b"EnumProperty"
            | b"ByteProperty"
            | b"StructProperty"
            | b"ArrayProperty"
            | b"SetProperty"
            | b"MapProperty"
    ) {
        return inspected_typed_value(bytes, kind, offset);
    }
    let value = sized_value(bytes, offset, 0).ok()?;
    match kind {
        b"Int8Property" => Some(i8::from_le_bytes(value.try_into().ok()?).to_string()),
        b"Int16Property" => Some(i16::from_le_bytes(value.try_into().ok()?).to_string()),
        b"IntProperty" => Some(i32::from_le_bytes(value.try_into().ok()?).to_string()),
        b"Int64Property" => Some(i64::from_le_bytes(value.try_into().ok()?).to_string()),
        b"UInt8Property" => Some(u8::from_le_bytes(value.try_into().ok()?).to_string()),
        b"UInt16Property" => Some(u16::from_le_bytes(value.try_into().ok()?).to_string()),
        b"UInt32Property" => Some(u32::from_le_bytes(value.try_into().ok()?).to_string()),
        b"UInt64Property" => Some(u64::from_le_bytes(value.try_into().ok()?).to_string()),
        b"FloatProperty" => Some(f32::from_le_bytes(value.try_into().ok()?).to_string()),
        b"DoubleProperty" => Some(f64::from_le_bytes(value.try_into().ok()?).to_string()),
        b"StrProperty" | b"NameProperty" | b"ObjectProperty" => string_value(value).ok(),
        b"TextProperty" => inspected_text(value)
            .or_else(|| Some(format!("localized text ({} bytes)", value.len()))),
        _ => None,
    }
}

fn prefixed_string(bytes: &[u8], offset: usize) -> Option<(String, usize)> {
    let length = i32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?);
    if length == 0 {
        return Some((String::new(), offset + 4));
    }
    let byte_length = if length < 0 {
        (length.unsigned_abs() as usize).checked_mul(2)?
    } else {
        length as usize
    };
    let end = offset.checked_add(4)?.checked_add(byte_length)?;
    let value = string_value(bytes.get(offset..end)?).ok()?;
    Some((value, end))
}

fn inspected_text(value: &[u8]) -> Option<String> {
    let history = *value.get(4)? as i8;
    match history {
        -1 => match value.get(5)? {
            0 if value.len() == 6 => Some(String::new()),
            1 => {
                let (text, end) = prefixed_string(value, 6)?;
                (end == value.len()).then_some(text)
            }
            _ => None,
        },
        0 => {
            let (_, offset) = prefixed_string(value, 5)?;
            let (_, offset) = prefixed_string(value, offset)?;
            let (text, end) = prefixed_string(value, offset)?;
            (end == value.len()).then_some(text)
        }
        _ => None,
    }
}

fn type_node(bytes: &[u8], offset: usize, depth: usize) -> Option<(&[u8], usize)> {
    if depth > 4 {
        return None;
    }
    let (name, after_name) = fstring(bytes, offset).ok()?;
    let children = u32::from_le_bytes(bytes.get(after_name..after_name + 4)?.try_into().ok()?);
    if children > 4 {
        return None;
    }
    let mut next = after_name + 4;
    for _ in 0..children {
        next = type_node(bytes, next, depth + 1)?.1;
    }
    Some((name, next))
}

fn inspected_typed_value(bytes: &[u8], kind: &[u8], offset: usize) -> Option<String> {
    let expected_children = match kind {
        b"EnumProperty" | b"MapProperty" => 2,
        _ => 1,
    };
    let children = u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?);
    if children != expected_children {
        return None;
    }
    let mut next = offset + 4;
    let (inner, after_first) = type_node(bytes, next, 0)?;
    next = after_first;
    let mut second = None;
    for _ in 1..children {
        let (name, after) = type_node(bytes, next, 0)?;
        second = Some(name);
        next = after;
    }
    // The last type node's zero child count precedes the value size and tag flags.
    let value_offset = next.checked_sub(4)?;
    let value = if kind == b"StructProperty" {
        sized_value(bytes, value_offset, 8)
            .or_else(|_| sized_value(bytes, value_offset, 0))
            .ok()?
    } else {
        sized_value(bytes, value_offset, 0).ok()?
    };
    let inner = std::str::from_utf8(inner).ok()?;
    let second = second.map(std::str::from_utf8).transpose().ok()?;
    match kind {
        b"EnumProperty" => {
            let (name, end) = fstring(value, 0).ok()?;
            (end == value.len()).then(|| {
                String::from_utf8_lossy(name.rsplit(|byte| *byte == b':').next().unwrap_or(name))
                    .into_owned()
            })
        }
        b"ByteProperty" => match value {
            [byte] => Some(byte.to_string()),
            _ => {
                let (name, end) = fstring(value, 0).ok()?;
                (end == value.len()).then(|| String::from_utf8_lossy(name).into_owned())
            }
        },
        b"StructProperty" if inner == "DateTime" => {
            let ticks = u64::from_le_bytes(value.try_into().ok()?);
            const UNIX_EPOCH_TICKS: i128 = 621_355_968_000_000_000;
            OffsetDateTime::from_unix_timestamp_nanos((i128::from(ticks) - UNIX_EPOCH_TICKS) * 100)
                .ok()?
                .format(&format_description!(
                    "[year]-[month]-[day] [hour]:[minute]:[second] UTC"
                ))
                .ok()
        }
        b"StructProperty" => Some(format!("{inner} ({} bytes)", value.len())),
        b"ArrayProperty" => {
            let count = u32::from_le_bytes(value.get(..4)?.try_into().ok()?);
            Some(format!("{count} elements of {inner}"))
        }
        b"SetProperty" | b"MapProperty" => {
            let removed = u32::from_le_bytes(value.get(..4)?.try_into().ok()?);
            if removed != 0 {
                return Some(format!("{inner} ({} bytes)", value.len()));
            }
            let count = u32::from_le_bytes(value.get(4..8)?.try_into().ok()?);
            if let Some(second) = second {
                Some(format!("{count} entries of {inner} → {second}"))
            } else {
                Some(format!("{count} elements of {inner}"))
            }
        }
        _ => None,
    }
}

fn fstring(bytes: &[u8], offset: usize) -> io::Result<(&[u8], usize)> {
    let length = bytes
        .get(offset..offset + 4)
        .and_then(|data| data.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| invalid("Truncated property name"))? as usize;
    if !(1..=256).contains(&length) {
        return Err(invalid("Invalid property name length"));
    }
    let end = offset + 4 + length;
    let value = bytes
        .get(offset + 4..end)
        .ok_or_else(|| invalid("Truncated property name"))?;
    let Some(name) = value.strip_suffix(&[0]) else {
        return Err(invalid("Unterminated property name"));
    };
    Ok((name, end))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn property(bytes: &[u8], name: &[u8], kind: &[u8]) -> io::Result<Option<usize>> {
    match find_property(bytes, name)? {
        Some((actual, offset, _)) if actual == kind => Ok(Some(offset)),
        Some(_) => Err(invalid("Unexpected property type")),
        None => Ok(None),
    }
}

fn sized_value(bytes: &[u8], offset: usize, marker: u8) -> io::Result<&[u8]> {
    let metadata = bytes
        .get(offset..offset + 9)
        .ok_or_else(|| invalid("Truncated property metadata"))?;
    if metadata[..4] != [0; 4] || metadata[8] != marker {
        return Err(invalid("Unsupported property encoding"));
    }
    let size = u32::from_le_bytes(metadata[4..8].try_into().unwrap()) as usize;
    bytes
        .get(offset + 9..offset + 9 + size)
        .ok_or_else(|| invalid("Truncated property value"))
}

fn string_value(bytes: &[u8]) -> io::Result<String> {
    let length = bytes
        .get(..4)
        .and_then(|data| data.try_into().ok())
        .map(i32::from_le_bytes)
        .ok_or_else(|| invalid("Truncated character name"))?;
    match length {
        1.. => {
            let length = length as usize;
            let data = bytes
                .get(4..)
                .filter(|data| data.len() == length)
                .and_then(|data| data.strip_suffix(&[0]))
                .ok_or_else(|| invalid("Invalid character name length"))?;
            std::str::from_utf8(data)
                .map(str::to_owned)
                .map_err(|_| invalid("Invalid character name text"))
        }
        ..=-1 => {
            let length = length.unsigned_abs() as usize;
            let byte_length = length
                .checked_mul(2)
                .ok_or_else(|| invalid("Invalid character name length"))?;
            let data = bytes
                .get(4..)
                .filter(|data| data.len() == byte_length)
                .ok_or_else(|| invalid("Invalid character name length"))?;
            let (chunks, []) = data.as_chunks::<2>() else {
                return Err(invalid("Invalid character name length"));
            };
            let mut units = chunks.iter().map(|chunk| u16::from_le_bytes(*chunk));
            if units.next_back() != Some(0) {
                return Err(invalid("Unterminated character name"));
            }
            String::from_utf16(&units.collect::<Vec<_>>())
                .map_err(|_| invalid("Invalid character name text"))
        }
        0 => Err(invalid("Empty character name")),
    }
}

pub fn death_stats(bytes: &[u8]) -> io::Result<DeathStats> {
    let count = numeric_property(bytes, b"m_DeathCounter")?;
    let play_seconds_at_last_death =
        property(bytes, b"m_TotalPlaytimeAtLastDeath", b"DoubleProperty")?
            .map(|offset| {
                let value: [u8; 8] = sized_value(bytes, offset, 0)?
                    .try_into()
                    .map_err(|_| invalid("Invalid last death playtime size"))?;
                Ok::<_, io::Error>(f64::from_le_bytes(value))
            })
            .transpose()?;
    Ok(DeathStats {
        count,
        play_seconds_at_last_death,
    })
}

/// Locates the complete serialized death property without relying on its offset.
pub fn death_property(bytes: &[u8]) -> io::Result<Option<Range<usize>>> {
    if !bytes.starts_with(b"GVAS") {
        return Err(invalid("Not an Unreal GVAS save"));
    }

    let mut found = None;
    for (start, _) in bytes
        .windows(NAME.len())
        .enumerate()
        .filter(|(_, window)| *window == NAME)
    {
        let type_start = start + NAME.len();
        let (property_type, metadata_start) = fstring(bytes, type_start)?;
        if property_type != b"BoolProperty" {
            return Err(invalid("Death flag has an unexpected property type"));
        }
        let metadata = bytes
            .get(metadata_start..metadata_start + 9)
            .ok_or_else(|| invalid("Truncated death property"))?;
        if metadata[..8] != [0; 8] || !matches!(metadata[8], 0 | 1 | 0x10) {
            return Err(invalid("Unsupported death property encoding"));
        }
        let end = metadata_start + 9;
        let (next_name, next_type_start) = fstring(bytes, end)?;
        if next_name != b"None" {
            let (next_type, _) = fstring(bytes, next_type_start)?;
            if !next_type.ends_with(b"Property") {
                return Err(invalid("Invalid property after death flag"));
            }
        }
        if found.replace(start..end).is_some() {
            return Err(invalid("Multiple death properties found"));
        }
    }
    Ok(found)
}

fn find_property<'a>(bytes: &'a [u8], name: &[u8]) -> io::Result<Option<(&'a [u8], usize, usize)>> {
    if !bytes.starts_with(b"GVAS") {
        return Err(invalid("Not an Unreal GVAS save"));
    }
    let width = name.len() + 5;
    let mut matches = bytes.windows(width).enumerate().filter(|(_, window)| {
        window[..4] == (name.len() as u32 + 1).to_le_bytes()
            && window[4..].starts_with(name)
            && window[width - 1] == 0
    });
    let Some((start, _)) = matches.next() else {
        return Ok(None);
    };
    if matches.next().is_some() {
        return Err(invalid("Multiple matching properties found"));
    }
    fstring(bytes, start + width).map(|(kind, offset)| Some((kind, offset, start)))
}

pub fn header_details(bytes: &[u8]) -> io::Result<HeaderDetails> {
    let difficulty = property(bytes, b"m_Difficulty", b"EnumProperty")?
        .map(|offset| {
            let (_, offset) = fstring(bytes, offset + 4)?;
            let (_, offset) = fstring(bytes, offset + 4)?;
            let (inner_kind, offset) = fstring(bytes, offset + 4)?;
            if inner_kind != b"ByteProperty" {
                return Err(invalid("Unexpected difficulty value type"));
            }
            let value = sized_value(bytes, offset, 0)?;
            let (name, _) = fstring(value, 0)?;
            let name = name.rsplit(|byte| *byte == b':').next().unwrap_or(name);
            std::str::from_utf8(name)
                .map(str::to_owned)
                .map_err(|_| invalid("Invalid difficulty value"))
        })
        .transpose()?;
    let level = property(bytes, b"m_Level", b"FloatProperty")?
        .map(|offset| {
            let value: [u8; 4] = sized_value(bytes, offset, 0)?
                .try_into()
                .map_err(|_| invalid("Invalid level size"))?;
            Ok::<_, io::Error>(f32::from_le_bytes(value))
        })
        .transpose()?;
    let last_save_ticks = property(bytes, b"m_LastSaveTimeUTC", b"StructProperty")?
        .map(|offset| {
            let (kind, offset) = fstring(bytes, offset + 4)?;
            if kind != b"DateTime" {
                return Err(invalid("Unexpected last save time struct"));
            }
            let (_, offset) = fstring(bytes, offset + 4)?;
            let value: [u8; 8] = sized_value(bytes, offset, 8)?
                .try_into()
                .map_err(|_| invalid("Invalid last save time size"))?;
            Ok::<_, io::Error>(u64::from_le_bytes(value))
        })
        .transpose()?;
    let play_seconds = numeric_property(bytes, b"m_TotalPlayTimeSeconds")?;
    Ok(HeaderDetails {
        difficulty,
        level,
        last_save_ticks,
        play_seconds,
    })
}

/// Reads the character's displayed name from its serialized header property.
pub fn character_name(bytes: &[u8]) -> io::Result<String> {
    if !bytes.starts_with(b"GVAS") {
        return Err(invalid("Not an Unreal GVAS save"));
    }
    let mut names = bytes
        .windows(CHARACTER_NAME.len())
        .enumerate()
        .filter(|(_, window)| *window == CHARACTER_NAME);
    let (start, _) = names
        .next()
        .ok_or_else(|| invalid("Character name property not found"))?;
    if names.next().is_some() {
        return Err(invalid("Multiple character name properties found"));
    }

    let (property_type, metadata_start) = fstring(bytes, start + CHARACTER_NAME.len())?;
    if property_type != b"StrProperty" {
        return Err(invalid("Character name has an unexpected property type"));
    }
    let metadata = bytes
        .get(metadata_start..metadata_start + 9)
        .ok_or_else(|| invalid("Truncated character name property"))?;
    let value_size = u32::from_le_bytes(
        metadata[4..8]
            .try_into()
            .map_err(|_| invalid("Truncated character name property"))?,
    ) as usize;
    if metadata[..4] != [0; 4] || metadata[8] != 0 {
        return Err(invalid("Unsupported character name property encoding"));
    }

    let value_start = metadata_start + 9;
    let value_end = value_start
        .checked_add(value_size)
        .ok_or_else(|| invalid("Invalid character name length"))?;
    let value = bytes
        .get(value_start..value_end)
        .ok_or_else(|| invalid("Truncated character name"))?;
    let name = string_value(value)?;
    if name.is_empty() {
        return Err(invalid("Empty character name"));
    }
    Ok(name)
}

fn numeric_property(bytes: &[u8], name: &[u8]) -> io::Result<Option<u64>> {
    let Some((kind, offset, _)) = find_property(bytes, name)? else {
        return Ok(None);
    };
    let value = sized_value(bytes, offset, 0)?;
    let number = match kind {
        b"IntProperty" => u32::from_le_bytes(
            value
                .try_into()
                .map_err(|_| invalid("Invalid integer size"))?,
        ) as u64,
        b"Int64Property" => u64::from_le_bytes(
            value
                .try_into()
                .map_err(|_| invalid("Invalid integer size"))?,
        ),
        _ => return Err(invalid("Unexpected integer property type")),
    };
    Ok(Some(number))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let mut bytes = b"GVAS\0\0\0\0".to_vec();
        bytes.extend_from_slice(NAME);
        bytes.extend_from_slice(b"\x0d\0\0\0BoolProperty\0\0\0\0\0\0\0\0\0\x01");
        bytes.extend_from_slice(b"\x0e\0\0\0m_VersionInfo\0\x0f\0\0\0StructProperty\0");
        bytes
    }

    fn named_fixture(value: &[u8]) -> Vec<u8> {
        let mut bytes = b"GVAS\0\0\0\0".to_vec();
        bytes.extend_from_slice(CHARACTER_NAME);
        bytes.extend_from_slice(b"\x0c\0\0\0StrProperty\0\0\0\0\0");
        bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(value);
        bytes
    }

    fn add_scalar(bytes: &mut Vec<u8>, name: &str, kind: &str, value: &[u8]) {
        for text in [name, kind] {
            bytes.extend_from_slice(&(text.len() as u32 + 1).to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
            bytes.push(0);
        }
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(value);
    }

    #[test]
    fn reads_header_statistics() {
        let mut bytes = b"GVAS".to_vec();
        for text in ["m_Difficulty", "EnumProperty"] {
            bytes.extend_from_slice(&(text.len() as u32 + 1).to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
            bytes.push(0);
        }
        bytes.extend_from_slice(&2u32.to_le_bytes());
        for text in ["ETQ2Difficulty", "/Script/TQ2Gameplay"] {
            bytes.extend_from_slice(&(text.len() as u32 + 1).to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
            bytes.push(0);
            if text == "ETQ2Difficulty" {
                bytes.extend_from_slice(&1u32.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(b"\x0d\0\0\0ByteProperty\0");
        let variant = b"\x19\0\0\0ETQ2Difficulty::Hardcore\0";
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&(variant.len() as u32).to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(variant);

        add_scalar(&mut bytes, "m_Level", "FloatProperty", &23f32.to_le_bytes());
        add_scalar(
            &mut bytes,
            "m_TotalPlayTimeSeconds",
            "Int64Property",
            &33_000u64.to_le_bytes(),
        );
        for text in ["m_LastSaveTimeUTC", "StructProperty"] {
            bytes.extend_from_slice(&(text.len() as u32 + 1).to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
            bytes.push(0);
        }
        bytes.extend_from_slice(&1u32.to_le_bytes());
        for text in ["DateTime", "/Script/CoreUObject"] {
            bytes.extend_from_slice(&(text.len() as u32 + 1).to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
            bytes.push(0);
            if text == "DateTime" {
                bytes.extend_from_slice(&1u32.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&8u32.to_le_bytes());
        bytes.push(8);
        bytes.extend_from_slice(&638_936_524_260_480_000u64.to_le_bytes());

        let details = header_details(&bytes).unwrap();
        assert_eq!(details.difficulty.as_deref(), Some("Hardcore"));
        assert_eq!(details.level, Some(23.0));
        assert_eq!(details.play_seconds, Some(33_000));
        assert_eq!(details.last_save_ticks, Some(638_936_524_260_480_000));

        let properties = inspect_properties(&bytes).unwrap();
        assert!(
            properties.iter().any(|property| {
                property.name == "m_Difficulty" && property.value == "Hardcore"
            })
        );
        assert!(properties.iter().any(|property| {
            property.name == "m_LastSaveTimeUTC" && property.value == "2025-09-16 20:47:06 UTC"
        }));
    }

    #[test]
    fn inspects_bool_and_collection_summaries() {
        fn name(bytes: &mut Vec<u8>, value: &str) {
            bytes.extend_from_slice(&(value.len() as u32 + 1).to_le_bytes());
            bytes.extend_from_slice(value.as_bytes());
            bytes.push(0);
        }

        fn collection(
            bytes: &mut Vec<u8>,
            name_value: &str,
            kind: &str,
            types: &[&str],
            value: &[u8],
        ) {
            name(bytes, name_value);
            name(bytes, kind);
            bytes.extend_from_slice(&(types.len() as u32).to_le_bytes());
            for kind in types {
                name(bytes, kind);
                bytes.extend_from_slice(&0u32.to_le_bytes());
            }
            bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
            bytes.push(0);
            bytes.extend_from_slice(value);
        }

        let mut bytes = b"GVAS".to_vec();
        name(&mut bytes, "m_HideHelmet");
        name(&mut bytes, "BoolProperty");
        bytes.extend_from_slice(&[0; 8]);
        bytes.push(0x10);
        collection(
            &mut bytes,
            "m_DataCategories",
            "ArrayProperty",
            &["IntProperty"],
            &[2, 0, 0, 0, 10, 0, 0, 0, 20, 0, 0, 0],
        );
        collection(
            &mut bytes,
            "m_SeenGoals",
            "SetProperty",
            &["NameProperty"],
            &[0, 0, 0, 0, 3, 0, 0, 0],
        );
        collection(
            &mut bytes,
            "m_VariableValues",
            "MapProperty",
            &["NameProperty", "IntProperty"],
            &[0, 0, 0, 0, 4, 0, 0, 0],
        );
        let properties = inspect_properties(&bytes).unwrap();
        assert_eq!(properties[0].value, "true");
        assert_eq!(properties[1].value, "2 elements of IntProperty");
        assert_eq!(properties[2].value, "3 elements of NameProperty");
        assert_eq!(
            properties[3].value,
            "4 entries of NameProperty → IntProperty"
        );
    }

    #[test]
    fn inspects_localized_text_source() {
        let mut value = vec![0; 4];
        value.push(0);
        for text in ["TQ2", "mastery.title", "Storm Caller"] {
            value.extend_from_slice(&(text.len() as i32 + 1).to_le_bytes());
            value.extend_from_slice(text.as_bytes());
            value.push(0);
        }
        let mut bytes = b"GVAS".to_vec();
        add_scalar(&mut bytes, "m_MasteryTitle", "TextProperty", &value);
        let properties = inspect_properties(&bytes).unwrap();
        assert_eq!(properties[0].value, "Storm Caller");
    }

    #[test]
    fn reads_optional_death_statistics() {
        let mut bytes = b"GVAS".to_vec();
        add_scalar(
            &mut bytes,
            "m_DeathCounter",
            "IntProperty",
            &2u32.to_le_bytes(),
        );
        add_scalar(
            &mut bytes,
            "m_TotalPlaytimeAtLastDeath",
            "DoubleProperty",
            &300.5f64.to_le_bytes(),
        );
        let stats = death_stats(&bytes).unwrap();
        assert_eq!(stats.count, Some(2));
        assert_eq!(stats.play_seconds_at_last_death, Some(300.5));
    }

    #[test]
    fn reads_character_name_from_header() {
        let bytes = named_fixture(b"\x06\0\0\0Hexen\0");
        assert_eq!(character_name(&bytes).unwrap(), "Hexen");
    }

    #[test]
    fn reads_utf16_character_name() {
        let mut value = (-6i32).to_le_bytes().to_vec();
        for unit in "Герой\0".encode_utf16() {
            value.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(character_name(&named_fixture(&value)).unwrap(), "Герой");
    }

    #[test]
    fn rejects_truncated_character_name() {
        let mut bytes = named_fixture(b"\x06\0\0\0Hexen\0");
        bytes.pop();
        assert!(character_name(&bytes).is_err());
    }

    #[test]
    fn locates_property_without_fixed_offset() {
        let mut bytes = fixture();
        let range = death_property(&bytes).unwrap().unwrap();
        assert_eq!(range.len(), 50);
        bytes.drain(range);
        assert_eq!(death_property(&bytes).unwrap(), None);
    }

    #[test]
    fn locates_death_property_with_current_bool_encoding() {
        let mut bytes = fixture();
        let marker = 8 + NAME.len() + b"\x0d\0\0\0BoolProperty\0".len() + 8;
        bytes[marker] = 0x10;
        let range = death_property(&bytes).unwrap().unwrap();
        assert_eq!(range.len(), 50);
        bytes.drain(range);
        assert_eq!(death_property(&bytes).unwrap(), None);
    }

    #[test]
    fn rejects_truncated_property() {
        let mut bytes = fixture();
        bytes.truncate(8 + NAME.len() + 4);
        assert!(death_property(&bytes).is_err());
    }

    #[test]
    fn rejects_multiple_properties() {
        let mut bytes = fixture();
        bytes.extend_from_slice(&fixture()[8..]);
        assert!(death_property(&bytes).is_err());
    }
}
