//! The looks a 1.12 client has textures for, from its `CharSections.dbc`
//! (skins, faces, hair) and `CharacterFacialHairStyles.dbc`. A character
//! with one it hasn't crashes the client that draws them, or their corpse:
//! so a new character's look is checked, and an old one's mended at login.

use ahash::AHashSet;

/// `CharSections`' kinds: a skin (by colour), a face (by skin colour), hair
/// (a style in a colour).
const SKIN: u8 = 0;
const FACE: u8 = 1;
const HAIR: u8 = 3;

/// A character's look, as `CMSG_CHAR_CREATE` and the store keep it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    pub skin: u8,
    pub face: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub facial_hair: u8,
}

#[derive(Debug, Default)]
pub struct Appearances {
    /// (race, gender, kind, variation, colour) of every section a player may
    /// have.
    sections: AHashSet<(u8, u8, u8, u8, u8)>,
    /// (race, gender, variation) of every facial hair.
    facial: AHashSet<(u8, u8, u8)>,
}

impl Appearances {
    pub fn add_section(&mut self, race: u8, gender: u8, kind: u8, variation: u8, colour: u8) {
        self.sections
            .insert((race, gender, kind, variation, colour));
    }

    pub fn add_facial_hair(&mut self, race: u8, gender: u8, variation: u8) {
        self.facial.insert((race, gender, variation));
    }

    /// Whether the client's tables are here to check against: without them
    /// (a bare test world) every look goes.
    pub fn is_loaded(&self) -> bool {
        !self.sections.is_empty()
    }

    pub fn valid(&self, race: u8, gender: u8, l: Look) -> bool {
        if !self.is_loaded() {
            return true;
        }
        let has = |kind, variation, colour| {
            self.sections
                .contains(&(race, gender, kind, variation, colour))
        };
        has(SKIN, 0, l.skin)
            && has(FACE, l.face, l.skin)
            && has(HAIR, l.hair_style, l.hair_color)
            && self.facial.contains(&(race, gender, l.facial_hair))
    }

    /// `l` with what the client has no texture for put back to the first
    /// it has, keeping the rest.
    pub fn mend(&self, race: u8, gender: u8, mut l: Look) -> Look {
        if !self.is_loaded() {
            return l;
        }
        let has = |kind, variation, colour| {
            self.sections
                .contains(&(race, gender, kind, variation, colour))
        };
        if !has(SKIN, 0, l.skin) {
            l.skin = 0;
        }
        if !has(FACE, l.face, l.skin) {
            l.face = 0;
        }
        if !has(HAIR, l.hair_style, l.hair_color) {
            if has(HAIR, l.hair_style, 0) {
                l.hair_color = 0;
            } else {
                l.hair_style = 0;
                l.hair_color = 0;
            }
        }
        if !self.facial.contains(&(race, gender, l.facial_hair)) {
            l.facial_hair = 0;
        }
        l
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tauren_and_gnome() -> Appearances {
        let mut a = Appearances::default();
        for (race, skins, faces, styles, colours) in [(6u8, 19u8, 5u8, 8u8, 3u8), (7, 5, 7, 7, 9)] {
            for skin in 0..skins {
                a.add_section(race, 0, SKIN, 0, skin);
                for face in 0..faces {
                    a.add_section(race, 0, FACE, face, skin);
                }
            }
            for style in 0..styles {
                for colour in 0..colours {
                    a.add_section(race, 0, HAIR, style, colour);
                }
            }
            for f in 0..5 {
                a.add_facial_hair(race, 0, f);
            }
        }
        a
    }

    /// The two looks a client crashed drawing (as corpses in Gurubashi
    /// Arena): a gnome with a sixth skin, a tauren with a fourth hair colour.
    #[test]
    fn a_look_the_client_has_no_texture_for_is_refused_and_mended() {
        let a = tauren_and_gnome();
        let look = |skin, face, hair_style, hair_color, facial_hair| Look {
            skin,
            face,
            hair_style,
            hair_color,
            facial_hair,
        };
        let gnome = look(5, 2, 4, 2, 0);
        assert!(!a.valid(7, 0, gnome));
        assert_eq!(a.mend(7, 0, gnome), look(0, 2, 4, 2, 0));
        let tauren = look(3, 3, 5, 4, 4);
        assert!(!a.valid(6, 0, tauren));
        assert_eq!(a.mend(6, 0, tauren), look(3, 3, 5, 0, 4));
        assert!(a.valid(6, 0, look(3, 3, 5, 2, 4)));
        assert!(a.valid(6, 0, a.mend(6, 0, look(30, 9, 9, 9, 9))));
        // Without the client's tables, anything goes.
        assert!(Appearances::default().valid(7, 0, gnome));
    }
}
