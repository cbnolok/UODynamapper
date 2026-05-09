//! # UO Animation Sequence Parser
//!
//! This module handles the parsing of `AnimationSequence.uop` for both
//! Classic Client (CC) and Enhanced Client (EC).
//! These files define how animation frames are sequenced into actions and directions.

crate::eyre_imports!();

use std::collections::HashMap;
use std::io::Cursor;
use byteorder::{LittleEndian, ReadBytesExt};

/// Represents a sequence of frames for a specific body animation action.
#[derive(Debug, Clone)]
pub struct AnimationSequence {
    pub body_id: u32,
    pub actions: HashMap<u16, ActionData>,
}

#[derive(Debug, Clone)]
pub struct ActionData {
    pub action_id: u16,
    pub directions: Vec<DirectionData>,
}

#[derive(Debug, Clone)]
pub struct DirectionData {
    pub frame_indices: Vec<u16>,
}

impl AnimationSequence {
    /// Attempts to parse the animation sequence, trying CC first, then EC.
    pub fn parse(data: &[u8], body_id: u32) -> eyre::Result<Self> {
        if let Ok(seq) = Self::parse_cc(data, body_id) {
            return Ok(seq);
        }
        Self::parse_ec(data, body_id)
    }

    /// Parses an AnimationSequence entry from the Classic Client (CC).
    pub fn parse_cc(data: &[u8], body_id: u32) -> eyre::Result<Self> {
        let mut reader = Cursor::new(data);
        
        // Skip header DWORD
        let _header = reader.read_u32::<LittleEndian>()?;

        // do { -BYTE -BYTE } while(v9^v10)
        loop {
            let b1 = reader.read_u8()?;
            let b2 = reader.read_u8()?;
            if b1 == b2 { break; }
            if reader.position() >= data.len() as u64 { eyre::bail!("CC parse: unexpected EOF in byte loop"); }
        }

        // do { -WORD -WORD } while(v15^v16)
        loop {
            let w1 = reader.read_u16::<LittleEndian>()?;
            let w2 = reader.read_u16::<LittleEndian>()?;
            if w1 == w2 { break; }
            if reader.position() >= data.len() as u64 { eyre::bail!("CC parse: unexpected EOF in word loop"); }
        }

        let mut actions = HashMap::new();
        let count = match reader.read_u32::<LittleEndian>() {
            Ok(c) => c,
            Err(_) => 0,
        };

        if count > 0 {
            for _ in 0..count {
                let action_id = reader.read_u32::<LittleEndian>()?; // Iteration
                let _unk1 = reader.read_u32::<LittleEndian>()?;
                let _unk2 = reader.read_u32::<LittleEndian>()?;
                let _unk3 = reader.read_u32::<LittleEndian>()?;

                // Nested byte loop
                loop {
                    let b1 = reader.read_u8()?;
                    let b2 = reader.read_u8()?;
                    if b1 == b2 { break; }
                }

                // Nested word loop
                loop {
                    let w1 = reader.read_u16::<LittleEndian>()?;
                    let w2 = reader.read_u16::<LittleEndian>()?;
                    if w1 == w2 { break; }
                }

                let sub_count = reader.read_u32::<LittleEndian>()?;
                let mut directions = Vec::with_capacity(sub_count as usize);

                if sub_count > 0 {
                    for _ in 0..sub_count {
                        let _unk_group = reader.read_u32::<LittleEndian>()?;
                        let x_count = reader.read_u32::<LittleEndian>()?;
                        
                        let mut frame_indices = Vec::with_capacity(x_count as usize);
                        if x_count > 0 {
                            for _ in 0..x_count {
                                let frame_idx = reader.read_u32::<LittleEndian>()?; 
                                frame_indices.push(frame_idx as u16);
                            }
                        }
                        directions.push(DirectionData { frame_indices });
                    }
                }

                actions.insert(action_id as u16, ActionData {
                    action_id: action_id as u16,
                    directions,
                });
            }

            // Final sub count loop
            let sub_count_final = match reader.read_u32::<LittleEndian>() {
                Ok(c) => c,
                Err(_) => 0,
            };
            if sub_count_final > 0 {
                for _ in 0..sub_count_final {
                    let _ = reader.read_u32::<LittleEndian>()?;
                }
            }
        }

        Ok(Self {
            body_id,
            actions,
        })
    }

    /// Parses an AnimationSequence entry from the Enhanced Client (EC).
    pub fn parse_ec(data: &[u8], body_id: u32) -> eyre::Result<Self> {
        let mut reader = Cursor::new(data);
        
        let mut actions = HashMap::new();
        let count = match reader.read_u32::<LittleEndian>() {
            Ok(c) => c,
            Err(_) => 0,
        };

        if count > 0 {
            for action_id in 0..count {
                let _b1 = reader.read_u8()?;
                let _b2 = reader.read_u8()?;
                let _unk_val = reader.read_i32::<LittleEndian>()?; // Can be -1
                let _unk1 = reader.read_u32::<LittleEndian>()?;

                // First SubCount loop
                let sub_count1 = reader.read_u32::<LittleEndian>()?;
                if sub_count1 > 0 {
                    for _ in 0..sub_count1 {
                        let _b = reader.read_u8()?;
                        let _d = reader.read_u32::<LittleEndian>()?;
                    }
                }

                // Second SubCount loop
                let sub_count2 = reader.read_u32::<LittleEndian>()?;
                if sub_count2 > 0 {
                    for _ in 0..sub_count2 {
                        let _b = reader.read_u8()?;
                        let _d1 = reader.read_u32::<LittleEndian>()?;
                        let _d2 = reader.read_u32::<LittleEndian>()?;
                    }
                }

                // Third SubCount loop (Directions)
                let sub_count3 = reader.read_u32::<LittleEndian>()?;
                let mut directions = Vec::with_capacity(sub_count3 as usize);
                if sub_count3 > 0 {
                    for _ in 0..sub_count3 {
                        let _b1 = reader.read_u8()?;
                        let _b2 = reader.read_u8()?;
                        let x_count = reader.read_u32::<LittleEndian>()?;
                        
                        let mut frame_indices = Vec::with_capacity(x_count as usize);
                        if x_count > 0 {
                            for _ in 0..x_count {
                                let frame_idx = reader.read_u32::<LittleEndian>()?;
                                frame_indices.push(frame_idx as u16);
                            }
                        }
                        directions.push(DirectionData { frame_indices });
                    }
                }

                actions.insert(action_id as u16, ActionData {
                    action_id: action_id as u16,
                    directions,
                });
            }
        }

        Ok(Self {
            body_id,
            actions,
        })
    }
}
