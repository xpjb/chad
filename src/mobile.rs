use std::ops::Range;

/// A rectangle in physical surface pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PhysicalRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PhysicalRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }
}

/// Selection direction is preserved as anchor and focus document byte offsets.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Selection {
    pub anchor: usize,
    pub focus: usize,
}

impl Selection {
    pub const fn new(anchor: usize, focus: usize) -> Self {
        Self { anchor, focus }
    }

    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.focus)..self.anchor.max(self.focus)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextInputOptions {
    pub multiline: bool,
    pub secure: bool,
}

/// The application supplies one bounded text window around the active selection.
/// Text and selection use global UTF-8 document byte offsets.
pub struct TextInputState<'a> {
    pub id: u64,
    /// Increment after every application-owned text or selection change, and when
    /// replacing or recentering the native mirror. Native edits keep this value.
    pub generation: u64,
    pub document_start: usize,
    pub text: &'a str,
    pub selection: Selection,
    pub field_rect: PhysicalRect,
    pub caret_rect: PhysicalRect,
    pub options: TextInputOptions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextSplice {
    /// Range in the document before this splice.
    pub range: Range<usize>,
    pub replacement: String,
}

/// One atomic native edit. Selection and composition refer to the document after
/// applying `splice`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextInputEvent {
    pub id: u64,
    pub generation: u64,
    pub splice: Option<TextSplice>,
    pub selection: Selection,
    pub composition: Option<Range<usize>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MobileViewport {
    pub visible: PhysicalRect,
    pub keyboard_visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MobileEvent {
    TextInput(TextInputEvent),
    ViewportChanged(MobileViewport),
}

#[cfg(any(target_os = "android", test))]
#[derive(Clone, Debug)]
pub(crate) struct TextMirror {
    pub id: u64,
    pub generation: u64,
    pub document_start: usize,
    pub text: String,
    pub selection: Selection,
    pub composition: Option<Range<usize>>,
    #[cfg_attr(test, allow(dead_code))]
    pub options: TextInputOptions,
}

#[cfg(any(target_os = "android", test))]
pub(crate) struct NativeTextSnapshot {
    pub id: u64,
    pub generation: u64,
    pub document_start: usize,
    pub text: String,
    pub selection_start_utf16: i32,
    pub selection_end_utf16: i32,
    pub composition_start_utf16: i32,
    pub composition_end_utf16: i32,
}

#[cfg(any(target_os = "android", test))]
pub(crate) fn utf16_to_byte(text: &str, offset: i32) -> usize {
    let target = offset.max(0) as usize;
    let mut utf16 = 0;
    for (byte, character) in text.char_indices() {
        let next = utf16 + character.len_utf16();
        if next > target {
            return byte;
        }
        utf16 = next;
    }
    text.len()
}

#[cfg(any(target_os = "android", test))]
pub(crate) fn byte_to_utf16(text: &str, byte: usize) -> Option<i32> {
    if byte > text.len() || !text.is_char_boundary(byte) {
        return None;
    }
    i32::try_from(text[..byte].encode_utf16().count()).ok()
}

#[cfg(any(target_os = "android", test))]
fn replacement(old: &str, new: &str) -> (Range<usize>, Range<usize>) {
    let old_bytes = old.as_bytes();
    let new_bytes = new.as_bytes();
    let mut prefix = 0;
    let shared = old_bytes.len().min(new_bytes.len());
    while prefix < shared && old_bytes[prefix] == new_bytes[prefix] {
        prefix += 1;
    }
    while prefix > 0 && (!old.is_char_boundary(prefix) || !new.is_char_boundary(prefix)) {
        prefix -= 1;
    }

    let mut suffix = 0;
    let old_remaining = old.len() - prefix;
    let new_remaining = new.len() - prefix;
    let shared = old_remaining.min(new_remaining);
    while suffix < shared
        && old_bytes[old.len() - suffix - 1] == new_bytes[new.len() - suffix - 1]
    {
        suffix += 1;
    }
    while suffix > 0
        && (!old.is_char_boundary(old.len() - suffix)
            || !new.is_char_boundary(new.len() - suffix))
    {
        suffix -= 1;
    }

    (prefix..old.len() - suffix, prefix..new.len() - suffix)
}

#[cfg(any(target_os = "android", test))]
pub(crate) fn apply_native_snapshot(
    mirror: &mut TextMirror,
    snapshot: NativeTextSnapshot,
) -> Option<TextInputEvent> {
    if snapshot.id != mirror.id
        || snapshot.generation != mirror.generation
        || snapshot.document_start != mirror.document_start
    {
        return None;
    }

    let local_anchor = utf16_to_byte(&snapshot.text, snapshot.selection_start_utf16);
    let local_focus = utf16_to_byte(&snapshot.text, snapshot.selection_end_utf16);
    let selection = Selection {
        anchor: mirror.document_start.checked_add(local_anchor)?,
        focus: mirror.document_start.checked_add(local_focus)?,
    };
    let composition = if snapshot.composition_start_utf16 >= 0
        && snapshot.composition_end_utf16 > snapshot.composition_start_utf16
    {
        let start = utf16_to_byte(&snapshot.text, snapshot.composition_start_utf16);
        let end = utf16_to_byte(&snapshot.text, snapshot.composition_end_utf16);
        Some(
            mirror.document_start.checked_add(start)?
                ..mirror.document_start.checked_add(end)?,
        )
    } else {
        None
    };

    let splice = if mirror.text == snapshot.text {
        None
    } else {
        let (old, new) = replacement(&mirror.text, &snapshot.text);
        Some(TextSplice {
            range: mirror.document_start.checked_add(old.start)?
                ..mirror.document_start.checked_add(old.end)?,
            replacement: snapshot.text[new].to_string(),
        })
    };

    if splice.is_none() && mirror.selection == selection && mirror.composition == composition {
        return None;
    }

    mirror.text = snapshot.text;
    mirror.selection = selection;
    mirror.composition = composition.clone();
    Some(TextInputEvent {
        id: mirror.id,
        generation: mirror.generation,
        splice,
        selection,
        composition,
    })
}

/// Extract at most `max_utf16` UTF-16 code units around a UTF-8 selection.
/// The complete selection is always retained, even when it exceeds the budget.
pub fn surrounding_text(
    text: &str,
    selection: Selection,
    max_utf16: usize,
) -> Option<(usize, &str, Selection)> {
    let range = selection.range();
    if range.end > text.len()
        || !text.is_char_boundary(range.start)
        || !text.is_char_boundary(range.end)
    {
        return None;
    }
    let selected_units = text[range.clone()].encode_utf16().count();
    let extra = max_utf16.saturating_sub(selected_units);
    let before_budget = extra * 4 / 5;
    let after_budget = extra - before_budget;

    let mut start = range.start;
    let mut used = 0;
    for (byte, character) in text[..range.start].char_indices().rev() {
        let units = character.len_utf16();
        if used + units > before_budget {
            break;
        }
        used += units;
        start = byte;
    }

    let mut end = range.end;
    let mut used = 0;
    for (offset, character) in text[range.end..].char_indices() {
        let units = character.len_utf16();
        if used + units > after_budget {
            break;
        }
        used += units;
        end = range.end + offset + character.len_utf8();
    }

    Some((start, &text[start..end], selection))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mirror(text: &str) -> TextMirror {
        TextMirror {
            id: 7,
            generation: 3,
            document_start: 100,
            text: text.to_string(),
            selection: Selection::new(100, 100),
            composition: None,
            options: TextInputOptions::default(),
        }
    }

    #[test]
    fn native_snapshot_preserves_unicode_and_rejects_stale_state() {
        let mut mirror = mirror("a🐻c");
        let event = apply_native_snapshot(
            &mut mirror,
            NativeTextSnapshot {
                id: 7,
                generation: 3,
                document_start: 100,
                text: "a世界c".into(),
                selection_start_utf16: 1,
                selection_end_utf16: 3,
                composition_start_utf16: 1,
                composition_end_utf16: 3,
            },
        )
        .unwrap();
        assert_eq!(event.splice, Some(TextSplice {
            range: 101..105,
            replacement: "世界".into(),
        }));
        assert_eq!(event.selection, Selection::new(101, 107));
        assert_eq!(event.composition, Some(101..107));

        assert!(apply_native_snapshot(
            &mut mirror,
            NativeTextSnapshot {
                id: 7,
                generation: 2,
                document_start: 100,
                text: "ignored".into(),
                selection_start_utf16: 0,
                selection_end_utf16: 0,
                composition_start_utf16: -1,
                composition_end_utf16: -1,
            },
        )
        .is_none());
        assert_eq!(mirror.text, "a世界c");
        assert_eq!(utf16_to_byte("a🐻b", 2), 1);
        assert_eq!(byte_to_utf16("a🐻b", 5), Some(3));
    }

    #[test]
    fn rapid_native_snapshots_chain_from_the_latest_mirror() {
        let mut mirror = mirror("a");
        let first = apply_native_snapshot(
            &mut mirror,
            NativeTextSnapshot {
                id: 7,
                generation: 3,
                document_start: 100,
                text: "ab".into(),
                selection_start_utf16: 2,
                selection_end_utf16: 2,
                composition_start_utf16: -1,
                composition_end_utf16: -1,
            },
        )
        .unwrap();
        assert_eq!(first.splice, Some(TextSplice {
            range: 101..101,
            replacement: "b".into(),
        }));

        let second = apply_native_snapshot(
            &mut mirror,
            NativeTextSnapshot {
                id: 7,
                generation: 3,
                document_start: 100,
                text: "abc".into(),
                selection_start_utf16: 3,
                selection_end_utf16: 3,
                composition_start_utf16: -1,
                composition_end_utf16: -1,
            },
        )
        .unwrap();
        assert_eq!(second.splice, Some(TextSplice {
            range: 102..102,
            replacement: "c".into(),
        }));
        assert_eq!(mirror.text, "abc");
    }

    #[test]
    fn duplicate_native_snapshots_are_dropped() {
        let mut mirror = mirror("hello");
        assert!(apply_native_snapshot(
            &mut mirror,
            NativeTextSnapshot {
                id: 7,
                generation: 3,
                document_start: 100,
                text: "hello".into(),
                selection_start_utf16: 0,
                selection_end_utf16: 0,
                composition_start_utf16: -1,
                composition_end_utf16: -1,
            },
        )
        .is_none());

        assert!(apply_native_snapshot(
            &mut mirror,
            NativeTextSnapshot {
                id: 7,
                generation: 3,
                document_start: 100,
                text: "hello".into(),
                selection_start_utf16: 2,
                selection_end_utf16: 2,
                composition_start_utf16: 1,
                composition_end_utf16: 3,
            },
        )
        .is_some());
        assert!(apply_native_snapshot(
            &mut mirror,
            NativeTextSnapshot {
                id: 7,
                generation: 3,
                document_start: 100,
                text: "hello".into(),
                selection_start_utf16: 2,
                selection_end_utf16: 2,
                composition_start_utf16: 1,
                composition_end_utf16: 3,
            },
        )
        .is_none());
    }

    #[test]
    fn surrounding_window_keeps_unicode_selection() {
        let text = "0123456789🐻abcdefghij";
        let bear = text.find('🐻').unwrap();
        let (start, window, selection) = surrounding_text(
            text,
            Selection::new(bear, bear + '🐻'.len_utf8()),
            8,
        )
        .unwrap();
        let local = selection.anchor - start..selection.focus - start;
        assert_eq!(&window[local], "🐻");
        assert_eq!(selection.anchor, bear);
        assert!(window.encode_utf16().count() <= 8);
    }
}
