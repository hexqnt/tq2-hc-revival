use std::{io, ops::Range};

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
        Some((actual, offset)) if actual == kind => Ok(Some(offset)),
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

fn find_property<'a>(bytes: &'a [u8], name: &[u8]) -> io::Result<Option<(&'a [u8], usize)>> {
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
    fstring(bytes, start + width).map(Some)
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
    let Some((kind, offset)) = find_property(bytes, name)? else {
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
