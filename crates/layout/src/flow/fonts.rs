//! Mapping CSS font families to parley font stacks.

use std::borrow::Cow;

use parley::fontique::{FallbackKey, FamilyId, Script};
use parley::{FontContext, FontFamily as FamilyStack, FontFamilyName as StackName};
use presswork_style::{FontFamilyName, GenericFamily};

use super::WebFonts;

/// The font each generic family resolves to.
///
/// fontconfig maps a generic family such as `serif` to a long list of fonts
/// covering many scripts, in an order that is not script-aware: Noto Serif
/// Bengali can come before Noto Serif Devanagari, and then wins characters
/// both contain, such as the danda (।). Browsers instead map each generic
/// family to one font and pick fallback fonts by script, so do the same.
#[derive(Debug, Default)]
pub(crate) struct GenericFonts {
    serif: Option<String>,
    sans_serif: Option<String>,
    monospace: Option<String>,
}

impl GenericFonts {
    pub fn resolve(fonts: &mut FontContext) -> Self {
        add_last_resort_fonts(fonts);
        let mut first = |generic| {
            let id = fonts.collection.generic_families(generic).next()?;
            fonts.collection.family_name(id).map(str::to_owned)
        };
        Self {
            serif: first(parley::GenericFamily::Serif),
            sans_serif: first(parley::GenericFamily::SansSerif),
            monospace: first(parley::GenericFamily::Monospace),
        }
    }

    /// Whether any generic family resolved to a font. If none did, the
    /// system has no usable fonts (or fontconfig could not be loaded).
    pub fn any(&self) -> bool {
        self.serif.is_some() || self.sans_serif.is_some() || self.monospace.is_some()
    }

    /// The parley font stack for a computed `font-family` list, with
    /// `@font-face` families mapped to their registered names.
    pub fn stack<'a>(
        &'a self,
        web: &'a WebFonts<'_>,
        families: &'a [FontFamilyName],
    ) -> FamilyStack<'a> {
        let names = families
            .iter()
            .map(|family| match family {
                FontFamilyName::Named(name) => StackName::named(web.name(name)),
                FontFamilyName::Generic(generic) => self.generic(*generic),
            })
            .collect();
        FamilyStack::List(Cow::Owned(names))
    }

    fn generic(&self, generic: GenericFamily) -> StackName<'_> {
        let (name, fallback) = match generic {
            GenericFamily::Serif => (&self.serif, parley::GenericFamily::Serif),
            GenericFamily::SansSerif => (&self.sans_serif, parley::GenericFamily::SansSerif),
            GenericFamily::Monospace => (&self.monospace, parley::GenericFamily::Monospace),
        };
        match name {
            Some(name) => StackName::named(name),
            None => fallback.into(),
        }
    }
}

/// Fonts with symbols (check marks, arrows, stars, boxes), tried in this
/// order for characters no other font has. Pure symbol fonts come first,
/// so that they do not take letters from fonts meant for them.
pub(crate) const SYMBOL_FONTS: [&str; 7] = [
    "Noto Sans Symbols 2",
    "Noto Sans Symbols",
    "Noto Sans Math",
    "Symbola",
    "Segoe UI Symbol",
    "Apple Symbols",
    "DejaVu Sans",
];

/// Color emoji fonts, the very last resort.
const EMOJI_FONTS: [&str; 3] = ["Noto Color Emoji", "Apple Color Emoji", "Segoe UI Emoji"];

/// Makes symbol fonts, and the Devanagari fallback font, the last resort
/// for every character.
///
/// parley picks fallback fonts by the script of the text around a
/// character, and characters of the Common script take the script of
/// their neighbours: a check mark (✔) in Latin text gets the Latin
/// fallback font, which does not have it, and drew as a missing glyph
/// although Noto Sans Symbols 2 was installed. So does a danda (।) after
/// Latin text. fontique adds the fallback fonts for Han script to every
/// query (for Common punctuation), after those for the text's script;
/// fonts appended to that list are tried after everything else.
fn add_last_resort_fonts(fonts: &mut FontContext) {
    let han = FallbackKey::new(Script::from_bytes(*b"Hani"), None);
    // Resolve the system's fallbacks for the key first: appending to an
    // unresolved key would stand in for them.
    let existing: Vec<FamilyId> = fonts.collection.fallback_families(han).collect();
    let mut families: Vec<FamilyId> = Vec::new();
    let mut add = |id: FamilyId| {
        if !existing.contains(&id) && !families.contains(&id) {
            families.push(id);
        }
    };
    for name in SYMBOL_FONTS {
        if let Some(id) = fonts.collection.family_id(name) {
            add(id);
        }
    }
    let devanagari = FallbackKey::new(Script::from_bytes(*b"Deva"), None);
    let devanagari: Vec<FamilyId> = fonts.collection.fallback_families(devanagari).collect();
    devanagari.into_iter().for_each(&mut add);
    for name in EMOJI_FONTS {
        if let Some(id) = fonts.collection.family_id(name) {
            add(id);
        }
    }
    if !families.is_empty() {
        fonts.collection.append_fallbacks(han, families.into_iter());
    }
}
