//! The world server: sessions, maps, and what they share.

pub mod battlegrounds;
pub mod champions;
pub mod cheer_sites;
pub mod cheer_speakers;
pub mod cities;
pub mod conn;
pub mod data;
pub mod elemental;
pub mod events;
pub mod fields;
pub mod fireworks;
pub mod ghost_magnet;
pub mod guid;
#[cfg(test)]
pub mod harness;
pub mod honor;
pub mod instances;
pub mod invasion;
pub mod jaina;
pub mod map;
pub mod omen;
pub mod registry;
pub mod session;
pub mod social;
pub mod store;
pub mod supervisor;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_ai5;
#[cfg(test)]
mod tests_ai5_conditions;
#[cfg(test)]
mod tests_ai5_instances;
#[cfg(test)]
mod tests_ai5_spells;
#[cfg(test)]
mod tests_answers2;
#[cfg(test)]
mod tests_arena;
#[cfg(test)]
mod tests_bg5;
#[cfg(test)]
mod tests_bg6;
#[cfg(test)]
mod tests_body;
#[cfg(test)]
mod tests_bosses;
#[cfg(test)]
mod tests_bugs1;
#[cfg(test)]
mod tests_bugs2;
#[cfg(test)]
mod tests_bugs3;
#[cfg(test)]
mod tests_bugs4;
#[cfg(test)]
mod tests_bugs5;
#[cfg(test)]
mod tests_bugs6;
#[cfg(test)]
mod tests_bugs7;
#[cfg(test)]
mod tests_calls11;
#[cfg(test)]
mod tests_charges;
#[cfg(test)]
mod tests_chat_fanout;
#[cfg(test)]
mod tests_cities;
#[cfg(test)]
mod tests_client_emu;
#[cfg(test)]
mod tests_combat;
#[cfg(test)]
mod tests_combat2;
#[cfg(test)]
mod tests_combat3;
#[cfg(test)]
mod tests_combat5;
#[cfg(test)]
mod tests_config;
#[cfg(test)]
mod tests_cooldowns;
#[cfg(test)]
mod tests_creatures4;
#[cfg(test)]
mod tests_dead_body;
#[cfg(test)]
mod tests_elites;
#[cfg(test)]
mod tests_entity_rest;
#[cfg(test)]
mod tests_entity_views;
#[cfg(test)]
mod tests_events;
#[cfg(test)]
mod tests_gear;
#[cfg(test)]
mod tests_ghost;
#[cfg(test)]
mod tests_gifts;
#[cfg(test)]
mod tests_guards;
#[cfg(test)]
mod tests_items3;
#[cfg(test)]
mod tests_loot;
#[cfg(test)]
mod tests_mangos_defaults;
#[cfg(test)]
mod tests_misc;
#[cfg(test)]
mod tests_misc2;
#[cfg(test)]
mod tests_net5;
#[cfg(test)]
mod tests_objects;
#[cfg(test)]
mod tests_order;
#[cfg(test)]
mod tests_own_guids;
#[cfg(test)]
mod tests_p10_ai_autocast;
#[cfg(test)]
mod tests_p10_ai_nav;
#[cfg(test)]
mod tests_p10_ai_owned;
#[cfg(test)]
mod tests_p10_ai_pet_combat;
#[cfg(test)]
mod tests_p10_ai_real;
#[cfg(test)]
mod tests_p10_combat_pets;
#[cfg(test)]
mod tests_p10_core_events;
#[cfg(test)]
mod tests_p10_core_loot;
#[cfg(test)]
mod tests_p10_core_power;
#[cfg(test)]
mod tests_p10_core_pvp;
#[cfg(test)]
mod tests_p10_core_real;
#[cfg(test)]
mod tests_p10_core_regen;
#[cfg(test)]
mod tests_p10_core_skills;
#[cfg(test)]
mod tests_p10_core_stats;
#[cfg(test)]
mod tests_p10_core_trade;
#[cfg(test)]
mod tests_p10_entity;
#[cfg(test)]
mod tests_p10_entity_real;
#[cfg(test)]
mod tests_p10_loader;
#[cfg(test)]
mod tests_p10_loader_pets;
#[cfg(test)]
mod tests_p10_movement_views;
#[cfg(test)]
mod tests_p10_player;
#[cfg(test)]
mod tests_p10_player_casts;
#[cfg(test)]
mod tests_p10_player_items;
#[cfg(test)]
mod tests_p10_player_objects;
#[cfg(test)]
mod tests_p10_player_quests;
#[cfg(test)]
mod tests_p10_player_real;
#[cfg(test)]
mod tests_p10_spell;
#[cfg(test)]
mod tests_p10_spell_cast;
#[cfg(test)]
mod tests_p10_world;
#[cfg(test)]
mod tests_p10_world_contact;
#[cfg(test)]
mod tests_p10_world_groups;
#[cfg(test)]
mod tests_p10_world_spells;
#[cfg(test)]
mod tests_p10_world_system;
#[cfg(test)]
mod tests_p11_entity;
#[cfg(test)]
mod tests_p11_entity_ai;
#[cfg(test)]
mod tests_p11_entity_more;
#[cfg(test)]
mod tests_p11_entity_real;
#[cfg(test)]
mod tests_p11_entity_spells;
#[cfg(test)]
mod tests_p11_entity_summons;
#[cfg(test)]
mod tests_p11a_resurrect;
#[cfg(test)]
mod tests_p11b_spells;
#[cfg(test)]
mod tests_p11c_checks;
#[cfg(test)]
mod tests_p11c_movement;
#[cfg(test)]
mod tests_p11c_range;
#[cfg(test)]
mod tests_p11d_pets;
#[cfg(test)]
mod tests_p11e_summons;
#[cfg(test)]
mod tests_p11f_movement;
#[cfg(test)]
mod tests_p11g_cooldowns;
#[cfg(test)]
mod tests_p11g_gossip;
#[cfg(test)]
mod tests_p11g_insignia;
#[cfg(test)]
mod tests_p11g_items;
#[cfg(test)]
mod tests_p11g_login_items;
#[cfg(test)]
mod tests_p11g_zones;
#[cfg(test)]
mod tests_p11h_stones;
#[cfg(test)]
mod tests_p11i_ai;
#[cfg(test)]
mod tests_p11i_av;
#[cfg(test)]
mod tests_p11i_misc;
#[cfg(test)]
mod tests_p11i_objects;
#[cfg(test)]
mod tests_p11i_spells;
#[cfg(test)]
mod tests_p11i_summons;
#[cfg(test)]
mod tests_p11i_views;
#[cfg(test)]
mod tests_p11i_world;
#[cfg(test)]
mod tests_p11j_control;
#[cfg(test)]
mod tests_p11j_loot;
#[cfg(test)]
mod tests_p11j_pets;
#[cfg(test)]
mod tests_p11j_quests;
#[cfg(test)]
mod tests_p11j_real;
#[cfg(test)]
mod tests_p11j_spells;
#[cfg(test)]
mod tests_p11j_world;
#[cfg(test)]
mod tests_p12_questions;
#[cfg(test)]
mod tests_p12f_auras;
#[cfg(test)]
mod tests_p12f_pets;
#[cfg(test)]
mod tests_p12f_real;
#[cfg(test)]
mod tests_p12f_sets;
#[cfg(test)]
mod tests_p3_ai_commands;
#[cfg(test)]
mod tests_p3_ai_events;
#[cfg(test)]
mod tests_p3_ai_pets;
#[cfg(test)]
mod tests_p3_ai_scripts;
#[cfg(test)]
mod tests_p3_aura2;
#[cfg(test)]
mod tests_p3_aura2_areas;
#[cfg(test)]
mod tests_p3_aura2_dispel;
#[cfg(test)]
mod tests_p3_aura2_dr;
#[cfg(test)]
mod tests_p3_aura2_immunity;
#[cfg(test)]
mod tests_p3_aura2_periodic;
#[cfg(test)]
mod tests_p3_aura2_real;
#[cfg(test)]
mod tests_p3_aura2_spells;
#[cfg(test)]
mod tests_p3_aura2_sync;
#[cfg(test)]
mod tests_p3_cast;
#[cfg(test)]
mod tests_p3_cast2;
#[cfg(test)]
mod tests_p3_cast3;
#[cfg(test)]
mod tests_p3_cast4;
#[cfg(test)]
mod tests_p3_cast5;
#[cfg(test)]
mod tests_p3_cast6;
#[cfg(test)]
mod tests_p3_cast7;
#[cfg(test)]
mod tests_p3_cast8;
#[cfg(test)]
mod tests_p3_cast9;
#[cfg(test)]
mod tests_p3_cast_class;
#[cfg(test)]
mod tests_p3_cast_kit;
#[cfg(test)]
mod tests_p3_cast_mage;
#[cfg(test)]
mod tests_p3_cast_priest;
#[cfg(test)]
mod tests_p3_combat;
#[cfg(test)]
mod tests_p3_combat10;
#[cfg(test)]
mod tests_p3_combat11;
#[cfg(test)]
mod tests_p3_combat2;
#[cfg(test)]
mod tests_p3_combat3;
#[cfg(test)]
mod tests_p3_combat3_assist;
#[cfg(test)]
mod tests_p3_combat3_block;
#[cfg(test)]
mod tests_p3_combat3_combo;
#[cfg(test)]
mod tests_p3_combat3_control;
#[cfg(test)]
mod tests_p3_combat3_defense;
#[cfg(test)]
mod tests_p3_combat3_disarm;
#[cfg(test)]
mod tests_p3_combat3_immune;
#[cfg(test)]
mod tests_p3_combat3_power;
#[cfg(test)]
mod tests_p3_combat3_regen;
#[cfg(test)]
mod tests_p3_combat3_share;
#[cfg(test)]
mod tests_p3_combat4;
#[cfg(test)]
mod tests_p3_combat5;
#[cfg(test)]
mod tests_p3_combat6;
#[cfg(test)]
mod tests_p3_combat7;
#[cfg(test)]
mod tests_p3_combat8;
#[cfg(test)]
mod tests_p3_combat9;
#[cfg(test)]
mod tests_p3_core_auction;
#[cfg(test)]
mod tests_p3_core_aura;
#[cfg(test)]
mod tests_p3_core_condition;
#[cfg(test)]
mod tests_p3_core_death;
#[cfg(test)]
mod tests_p3_core_guild;
#[cfg(test)]
mod tests_p3_core_instance;
#[cfg(test)]
mod tests_p3_core_inventory;
#[cfg(test)]
mod tests_p3_core_misc;
#[cfg(test)]
mod tests_p3_core_party;
#[cfg(test)]
mod tests_p3_core_pvp;
#[cfg(test)]
mod tests_p3_core_reputation;
#[cfg(test)]
mod tests_p3_core_social;
#[cfg(test)]
mod tests_p3_core_stone;
#[cfg(test)]
mod tests_p3_core_strat;
#[cfg(test)]
mod tests_p3_core_trade;
#[cfg(test)]
mod tests_p3_creature2;
#[cfg(test)]
mod tests_p3_entity;
#[cfg(test)]
mod tests_p3_entity_effects;
#[cfg(test)]
mod tests_p3_entity_env;
#[cfg(test)]
mod tests_p3_entity_eventai;
#[cfg(test)]
mod tests_p3_entity_mob;
#[cfg(test)]
mod tests_p3_entity_objects;
#[cfg(test)]
mod tests_p3_entity_pets;
#[cfg(test)]
mod tests_p3_entity_player;
#[cfg(test)]
mod tests_p3_entity_scripts;
#[cfg(test)]
mod tests_p3_items;
#[cfg(test)]
mod tests_p3_items_loot;
#[cfg(test)]
mod tests_p3_items_professions;
#[cfg(test)]
mod tests_p3_items_quests;
#[cfg(test)]
mod tests_p3_loader2_items;
#[cfg(test)]
mod tests_p3_loader2_mobs;
#[cfg(test)]
mod tests_p3_loader2_pets;
#[cfg(test)]
mod tests_p3_loader2_procs;
#[cfg(test)]
mod tests_p3_loader2_quests;
#[cfg(test)]
mod tests_p3_loader2_spells;
#[cfg(test)]
mod tests_p3_loader2_world;
#[cfg(test)]
mod tests_p3_loader_play;
#[cfg(test)]
mod tests_p3_loader_spells;
#[cfg(test)]
mod tests_p3_loader_world;
#[cfg(test)]
mod tests_p3_network;
#[cfg(test)]
mod tests_p3_network_auth;
#[cfg(test)]
mod tests_p3_network_codec;
#[cfg(test)]
mod tests_p3_network_messages;
#[cfg(test)]
mod tests_p3_network_npc;
#[cfg(test)]
mod tests_p3_network_pets;
#[cfg(test)]
mod tests_p3_network_real;
#[cfg(test)]
mod tests_p3_network_wire;
#[cfg(test)]
mod tests_p3_player;
#[cfg(test)]
mod tests_p3_player2_ammo;
#[cfg(test)]
mod tests_p3_player2_chars;
#[cfg(test)]
mod tests_p3_player2_gear;
#[cfg(test)]
mod tests_p3_player2_gm;
#[cfg(test)]
mod tests_p3_player2_home;
#[cfg(test)]
mod tests_p3_player2_npc;
#[cfg(test)]
mod tests_p3_player2_quests;
#[cfg(test)]
mod tests_p3_player2_taxi;
#[cfg(test)]
mod tests_p3_player_chat;
#[cfg(test)]
mod tests_p3_player_class;
#[cfg(test)]
mod tests_p3_player_death;
#[cfg(test)]
mod tests_p3_player_honor;
#[cfg(test)]
mod tests_p3_player_items;
#[cfg(test)]
mod tests_p3_player_pet;
#[cfg(test)]
mod tests_p3_player_social;
#[cfg(test)]
mod tests_p3_player_stats;
#[cfg(test)]
mod tests_p3_player_world;
#[cfg(test)]
mod tests_p3_scripts;
#[cfg(test)]
mod tests_p3_scripts_dungeons;
#[cfg(test)]
mod tests_p3_scripts_mc;
#[cfg(test)]
mod tests_p3_scripts_raids;
#[cfg(test)]
mod tests_p3_spells;
#[cfg(test)]
mod tests_p3_spells_auras;
#[cfg(test)]
mod tests_p3_spells_damage;
#[cfg(test)]
mod tests_p3_spells_kit;
#[cfg(test)]
mod tests_p3_world_bg;
#[cfg(test)]
mod tests_p3_world_env;
#[cfg(test)]
mod tests_p3_world_events;
#[cfg(test)]
mod tests_p3_world_pvp;
#[cfg(test)]
mod tests_p3_world_travel;
#[cfg(test)]
mod tests_p4_core_ap;
#[cfg(test)]
mod tests_p4_core_combat;
#[cfg(test)]
mod tests_p4_core_entity;
#[cfg(test)]
mod tests_p4_core_fields;
#[cfg(test)]
mod tests_p4_core_mob;
#[cfg(test)]
mod tests_p4_core_motion;
#[cfg(test)]
mod tests_p4_core_pen;
#[cfg(test)]
mod tests_p4_core_skills;
#[cfg(test)]
mod tests_p4_core_spell;
#[cfg(test)]
mod tests_p4_core_stats;
#[cfg(test)]
mod tests_p4_core_versus;
#[cfg(test)]
mod tests_p4_core_walk;
#[cfg(test)]
mod tests_p4_core_wand;
#[cfg(test)]
mod tests_p4_items_av;
#[cfg(test)]
mod tests_p4_items_bags;
#[cfg(test)]
mod tests_p4_items_conditions;
#[cfg(test)]
mod tests_p4_items_crafts;
#[cfg(test)]
mod tests_p4_items_events;
#[cfg(test)]
mod tests_p4_items_gear;
#[cfg(test)]
mod tests_p4_items_gear2;
#[cfg(test)]
mod tests_p4_items_honor;
#[cfg(test)]
mod tests_p4_items_objects;
#[cfg(test)]
mod tests_p4_items_quests;
#[cfg(test)]
mod tests_p4_items_quests2;
#[cfg(test)]
mod tests_p4_items_use;
#[cfg(test)]
mod tests_p4_items_use2;
#[cfg(test)]
mod tests_p4_scripts_bosses;
#[cfg(test)]
mod tests_p4_scripts_brd;
#[cfg(test)]
mod tests_p4_scripts_dungeons;
#[cfg(test)]
mod tests_p4_scripts_dungeons2;
#[cfg(test)]
mod tests_p4_scripts_dungeons3;
#[cfg(test)]
mod tests_p4_scripts_gnomes;
#[cfg(test)]
mod tests_p4_scripts_instances;
#[cfg(test)]
mod tests_p4_scripts_strat;
#[cfg(test)]
mod tests_p4_world;
#[cfg(test)]
mod tests_p4_world_guardians;
#[cfg(test)]
mod tests_p4_world_items;
#[cfg(test)]
mod tests_p4_world_loot;
#[cfg(test)]
mod tests_p4_world_net;
#[cfg(test)]
mod tests_p4_world_real;
#[cfg(test)]
mod tests_p4_world_rewards;
#[cfg(test)]
mod tests_p4_world_scripts;
#[cfg(test)]
mod tests_p4_world_spells;
#[cfg(test)]
mod tests_p5_ai_pet_targets;
#[cfg(test)]
mod tests_p5_ai_pets;
#[cfg(test)]
mod tests_p5_ai_scripts;
#[cfg(test)]
mod tests_p5_ai_summons;
#[cfg(test)]
mod tests_p5_combat;
#[cfg(test)]
mod tests_p5_combat_damage;
#[cfg(test)]
mod tests_p5_combat_defense;
#[cfg(test)]
mod tests_p5_combat_engage;
#[cfg(test)]
mod tests_p5_combat_feedback;
#[cfg(test)]
mod tests_p5_combat_heal;
#[cfg(test)]
mod tests_p5_combat_kills;
#[cfg(test)]
mod tests_p5_combat_procs;
#[cfg(test)]
mod tests_p5_combat_rewards;
#[cfg(test)]
mod tests_p5_combat_skills;
#[cfg(test)]
mod tests_p5_combat_state;
#[cfg(test)]
mod tests_p5_combat_taken;
#[cfg(test)]
mod tests_p5_combat_zone;
#[cfg(test)]
mod tests_p5_data_aura;
#[cfg(test)]
mod tests_p5_data_carrier;
#[cfg(test)]
mod tests_p5_data_dr;
#[cfg(test)]
mod tests_p5_data_gear;
#[cfg(test)]
mod tests_p5_data_periodic;
#[cfg(test)]
mod tests_p5_data_procs;
#[cfg(test)]
mod tests_p5_data_real;
#[cfg(test)]
mod tests_p5_data_single;
#[cfg(test)]
mod tests_p5_data_threat;
#[cfg(test)]
mod tests_p5_player;
#[cfg(test)]
mod tests_p5_player_items;
#[cfg(test)]
mod tests_p5_player_misc;
#[cfg(test)]
mod tests_p5_player_npc;
#[cfg(test)]
mod tests_p5_player_pets;
#[cfg(test)]
mod tests_p5_player_quests;
#[cfg(test)]
mod tests_p5_player_real;
#[cfg(test)]
mod tests_p5_player_vendor;
#[cfg(test)]
mod tests_p5_spell;
#[cfg(test)]
mod tests_p5_spell_blocks;
#[cfg(test)]
mod tests_p5_spell_channel;
#[cfg(test)]
mod tests_p5_spell_checks;
#[cfg(test)]
mod tests_p5_spell_cost;
#[cfg(test)]
mod tests_p5_spell_dbc;
#[cfg(test)]
mod tests_p5_spell_effects;
#[cfg(test)]
mod tests_p5_spell_enchant;
#[cfg(test)]
mod tests_p5_spell_holiday;
#[cfg(test)]
mod tests_p5_spell_hunter;
#[cfg(test)]
mod tests_p5_spell_items;
#[cfg(test)]
mod tests_p5_spell_paladin;
#[cfg(test)]
mod tests_p5_spell_procs;
#[cfg(test)]
mod tests_p5_spell_swiftmend;
#[cfg(test)]
mod tests_p5_spell_targets;
#[cfg(test)]
mod tests_p5_spell_warlock;
#[cfg(test)]
mod tests_p6_ai_melee;
#[cfg(test)]
mod tests_p6_ai_ranged;
#[cfg(test)]
mod tests_p6_ai_scripts;
#[cfg(test)]
mod tests_p6_ai_spells;
#[cfg(test)]
mod tests_p6_combat_classes;
#[cfg(test)]
mod tests_p6_combat_damage;
#[cfg(test)]
mod tests_p6_combat_procs;
#[cfg(test)]
mod tests_p6_combat_state;
#[cfg(test)]
mod tests_p6_combat_taken;
#[cfg(test)]
mod tests_p6_combat_weapons;
#[cfg(test)]
mod tests_p6_entity;
#[cfg(test)]
mod tests_p6_entity_effects;
#[cfg(test)]
mod tests_p6_entity_mob;
#[cfg(test)]
mod tests_p6_loader_items;
#[cfg(test)]
mod tests_p6_loader_spells;
#[cfg(test)]
mod tests_p6_loader_world;
#[cfg(test)]
mod tests_p6_misc;
#[cfg(test)]
mod tests_p6_misc_ai;
#[cfg(test)]
mod tests_p6_misc_auras;
#[cfg(test)]
mod tests_p6_misc_duel;
#[cfg(test)]
mod tests_p6_misc_pets;
#[cfg(test)]
mod tests_p6_misc_rewards;
#[cfg(test)]
mod tests_p6_player;
#[cfg(test)]
mod tests_p6_player_copies;
#[cfg(test)]
mod tests_p6_player_corpses;
#[cfg(test)]
mod tests_p6_player_items;
#[cfg(test)]
mod tests_p6_player_pets;
#[cfg(test)]
mod tests_p6_player_real;
#[cfg(test)]
mod tests_p6_scripts;
#[cfg(test)]
mod tests_p6_spell;
#[cfg(test)]
mod tests_p6_spell_cast;
#[cfg(test)]
mod tests_p6_spell_combat;
#[cfg(test)]
mod tests_p6_spell_effects;
#[cfg(test)]
mod tests_p6_spell_engage;
#[cfg(test)]
mod tests_p6_spell_power;
#[cfg(test)]
mod tests_p6_spell_range;
#[cfg(test)]
mod tests_p6_world;
#[cfg(test)]
mod tests_p6_world_events;
#[cfg(test)]
mod tests_p6_world_scripts;
#[cfg(test)]
mod tests_p6_world_sight;
#[cfg(test)]
mod tests_p6_world_system;
#[cfg(test)]
mod tests_p6_world_trade;
#[cfg(test)]
mod tests_p6_wspell;
#[cfg(test)]
mod tests_p6_wspell_areas;
#[cfg(test)]
mod tests_p6_wspell_groups;
#[cfg(test)]
mod tests_p6_wspell_locations;
#[cfg(test)]
mod tests_p6_wspell_reception;
#[cfg(test)]
mod tests_p6_wspell_requirements;
#[cfg(test)]
mod tests_p6_wspell_units;
#[cfg(test)]
mod tests_p7_ai_motion;
#[cfg(test)]
mod tests_p7_ai_pets;
#[cfg(test)]
mod tests_p7_ai_scripts;
#[cfg(test)]
mod tests_p7_auras;
#[cfg(test)]
mod tests_p7_auras_periodic;
#[cfg(test)]
mod tests_p7_auras_real;
#[cfg(test)]
mod tests_p7_auras_rolls;
#[cfg(test)]
mod tests_p7_auras_stealth;
#[cfg(test)]
mod tests_p7_combat;
#[cfg(test)]
mod tests_p7_combat_engage;
#[cfg(test)]
mod tests_p7_combat_feign;
#[cfg(test)]
mod tests_p7_combat_flee;
#[cfg(test)]
mod tests_p7_combat_leash;
#[cfg(test)]
mod tests_p7_combat_rewards;
#[cfg(test)]
mod tests_p7_combat_zone;
#[cfg(test)]
mod tests_p7_core;
#[cfg(test)]
mod tests_p7_core_auction;
#[cfg(test)]
mod tests_p7_core_bg;
#[cfg(test)]
mod tests_p7_core_loot;
#[cfg(test)]
mod tests_p7_core_motion;
#[cfg(test)]
mod tests_p7_core_objects;
#[cfg(test)]
mod tests_p7_creature_charm;
#[cfg(test)]
mod tests_p7_creature_critters;
#[cfg(test)]
mod tests_p7_creature_entry;
#[cfg(test)]
mod tests_p7_creature_events;
#[cfg(test)]
mod tests_p7_creature_immunity;
#[cfg(test)]
mod tests_p7_creature_misc;
#[cfg(test)]
mod tests_p7_creature_motion;
#[cfg(test)]
mod tests_p7_entity;
#[cfg(test)]
mod tests_p7_entity_loot;
#[cfg(test)]
mod tests_p7_entity_mob;
#[cfg(test)]
mod tests_p7_entity_objects;
#[cfg(test)]
mod tests_p7_loader_effects;
#[cfg(test)]
mod tests_p7_loader_items;
#[cfg(test)]
mod tests_p7_loader_pets;
#[cfg(test)]
mod tests_p7_loader_quests;
#[cfg(test)]
mod tests_p7_loader_scripts;
#[cfg(test)]
mod tests_p7_loader_spells;
#[cfg(test)]
mod tests_p7_loader_world;
#[cfg(test)]
mod tests_p7_movement_acks;
#[cfg(test)]
mod tests_p7_movement_charge;
#[cfg(test)]
mod tests_p7_movement_fear;
#[cfg(test)]
mod tests_p7_movement_ground;
#[cfg(test)]
mod tests_p7_movement_wire;
#[cfg(test)]
mod tests_p7_pets;
#[cfg(test)]
mod tests_p7_pets_real;
#[cfg(test)]
mod tests_p7_player;
#[cfg(test)]
mod tests_p7_player_bgs;
#[cfg(test)]
mod tests_p7_player_gm;
#[cfg(test)]
mod tests_p7_player_items;
#[cfg(test)]
mod tests_p7_player_pets;
#[cfg(test)]
mod tests_p7_player_social;
#[cfg(test)]
mod tests_p7_player_spells;
#[cfg(test)]
mod tests_p7_scripts;
#[cfg(test)]
mod tests_p7_scripts_dungeons;
#[cfg(test)]
mod tests_p7_scripts_nightmare;
#[cfg(test)]
mod tests_p7_scripts_quests;
#[cfg(test)]
mod tests_p7_spell_caster;
#[cfg(test)]
mod tests_p7_spell_channel;
#[cfg(test)]
mod tests_p7_spell_checks;
#[cfg(test)]
mod tests_p7_spell_effects;
#[cfg(test)]
mod tests_p7_spell_forms;
#[cfg(test)]
mod tests_p7_spell_groups;
#[cfg(test)]
mod tests_p7_spell_proc_sides;
#[cfg(test)]
mod tests_p7_spell_procs;
#[cfg(test)]
mod tests_p7_spell_scripts;
#[cfg(test)]
mod tests_p7_spell_teleport;
#[cfg(test)]
mod tests_p7_spell_totems;
#[cfg(test)]
mod tests_p7_world_combat;
#[cfg(test)]
mod tests_p7_world_nav;
#[cfg(test)]
mod tests_p7_world_sight;
#[cfg(test)]
mod tests_p7_world_spells;
#[cfg(test)]
mod tests_p7_world_system;
#[cfg(test)]
mod tests_p8_ai_motion;
#[cfg(test)]
mod tests_p8_ai_pets;
#[cfg(test)]
mod tests_p8_ai_retreat;
#[cfg(test)]
mod tests_p8_ai_scripts;
#[cfg(test)]
mod tests_p8_ai_spells;
#[cfg(test)]
mod tests_p8_auras;
#[cfg(test)]
mod tests_p8_combat;
#[cfg(test)]
mod tests_p8_combat_block;
#[cfg(test)]
mod tests_p8_combat_control;
#[cfg(test)]
mod tests_p8_combat_defense;
#[cfg(test)]
mod tests_p8_combat_disarm;
#[cfg(test)]
mod tests_p8_combat_elemental;
#[cfg(test)]
mod tests_p8_combat_extra;
#[cfg(test)]
mod tests_p8_combat_heal;
#[cfg(test)]
mod tests_p8_combat_origin;
#[cfg(test)]
mod tests_p8_combat_posture;
#[cfg(test)]
mod tests_p8_combat_reactive;
#[cfg(test)]
mod tests_p8_combat_share;
#[cfg(test)]
mod tests_p8_combat_state;
#[cfg(test)]
mod tests_p8_creature;
#[cfg(test)]
mod tests_p8_creature2;
#[cfg(test)]
mod tests_p8_creature3;
#[cfg(test)]
mod tests_p8_entity;
#[cfg(test)]
mod tests_p8_entity_mob;
#[cfg(test)]
mod tests_p8_entity_objects;
#[cfg(test)]
mod tests_p8_entity_pvp;
#[cfg(test)]
mod tests_p8_loader_combat;
#[cfg(test)]
mod tests_p8_loader_limits;
#[cfg(test)]
mod tests_p8_loader_mods;
#[cfg(test)]
mod tests_p8_loader_procs;
#[cfg(test)]
mod tests_p8_loader_rogue;
#[cfg(test)]
mod tests_p8_loader_spells;
#[cfg(test)]
mod tests_p8_movement_client;
#[cfg(test)]
mod tests_p8_movement_control;
#[cfg(test)]
mod tests_p8_movement_copy;
#[cfg(test)]
mod tests_p8_movement_ground;
#[cfg(test)]
mod tests_p8_movement_inbound;
#[cfg(test)]
mod tests_p8_pets_control;
#[cfg(test)]
mod tests_p8_pets_dummy;
#[cfg(test)]
mod tests_p8_pets_owner;
#[cfg(test)]
mod tests_p8_player_bank;
#[cfg(test)]
mod tests_p8_player_gear;
#[cfg(test)]
mod tests_p8_player_login;
#[cfg(test)]
mod tests_p8_player_pets;
#[cfg(test)]
mod tests_p8_player_quests;
#[cfg(test)]
mod tests_p8_player_reputation;
#[cfg(test)]
mod tests_p8_player_session;
#[cfg(test)]
mod tests_p8_player_taxi;
#[cfg(test)]
mod tests_p8_player_vendor;
#[cfg(test)]
mod tests_p8_scripts;
#[cfg(test)]
mod tests_p8_scripts_av;
#[cfg(test)]
mod tests_p8_scripts_av2;
#[cfg(test)]
mod tests_p8_spell_amounts;
#[cfg(test)]
mod tests_p8_spell_effects;
#[cfg(test)]
mod tests_p8_spell_funnel;
#[cfg(test)]
mod tests_p8_spell_mounts;
#[cfg(test)]
mod tests_p8_spell_real;
#[cfg(test)]
mod tests_p8_spell_resists;
#[cfg(test)]
mod tests_p8_spell_stealth;
#[cfg(test)]
mod tests_p8_spell_wounds;
#[cfg(test)]
mod tests_p8_world_combat;
#[cfg(test)]
mod tests_p8_world_sight;
#[cfg(test)]
mod tests_p8_world_spells;
#[cfg(test)]
mod tests_p8_world_system;
#[cfg(test)]
mod tests_p8_world_units;
#[cfg(test)]
mod tests_p9_core_class;
#[cfg(test)]
mod tests_p9_core_class_real;
#[cfg(test)]
mod tests_p9_core_entity;
#[cfg(test)]
mod tests_p9_core_motion;
#[cfg(test)]
mod tests_p9_core_pets;
#[cfg(test)]
mod tests_p9_core_power;
#[cfg(test)]
mod tests_p9_core_scripts;
#[cfg(test)]
mod tests_p9_core_terrain;
#[cfg(test)]
mod tests_p9_core_trade;
#[cfg(test)]
mod tests_p9_creatures;
#[cfg(test)]
mod tests_p9_creatures_pets;
#[cfg(test)]
mod tests_p9_creatures_real;
#[cfg(test)]
mod tests_p9_entity;
#[cfg(test)]
mod tests_p9_entity_pets;
#[cfg(test)]
mod tests_p9_entity_real;
#[cfg(test)]
mod tests_p9_player_bank;
#[cfg(test)]
mod tests_p9_player_bg;
#[cfg(test)]
mod tests_p9_player_control;
#[cfg(test)]
mod tests_p9_player_items;
#[cfg(test)]
mod tests_p9_player_locks;
#[cfg(test)]
mod tests_p9_player_quests;
#[cfg(test)]
mod tests_p9_player_social;
#[cfg(test)]
mod tests_p9_player_spells_real;
#[cfg(test)]
mod tests_p9_spells_cast;
#[cfg(test)]
mod tests_p9_spells_contact;
#[cfg(test)]
mod tests_p9_spells_effects;
#[cfg(test)]
mod tests_p9_spells_real;
#[cfg(test)]
mod tests_p9_spells_world;
#[cfg(test)]
mod tests_p9_world_data;
#[cfg(test)]
mod tests_p9_world_misc;
#[cfg(test)]
mod tests_p9_world_nav;
#[cfg(test)]
mod tests_p9_world_sight;
#[cfg(test)]
mod tests_p9_world_spells;
#[cfg(test)]
mod tests_p9_world_spells2;
#[cfg(test)]
mod tests_p9_world_spells3;
#[cfg(test)]
mod tests_p9_world_system;
#[cfg(test)]
mod tests_parity_combat;
#[cfg(test)]
mod tests_parity_misc;
#[cfg(test)]
mod tests_parity_motion;
#[cfg(test)]
mod tests_parity_scripts;
#[cfg(test)]
mod tests_parity_spells;
#[cfg(test)]
mod tests_parity_world;
#[cfg(test)]
mod tests_perf2;
#[cfg(test)]
mod tests_pets;
#[cfg(test)]
mod tests_pets2;
#[cfg(test)]
mod tests_pvp;
#[cfg(test)]
mod tests_pvp2;
#[cfg(test)]
mod tests_pvp3;
#[cfg(test)]
mod tests_pvp4;
#[cfg(test)]
mod tests_pvp5;
#[cfg(test)]
mod tests_ready;
#[cfg(test)]
mod tests_review2;
#[cfg(test)]
mod tests_review3;
#[cfg(test)]
mod tests_review4;
#[cfg(test)]
mod tests_review5_data;
#[cfg(test)]
mod tests_review5_maps;
#[cfg(test)]
mod tests_review5_social;
#[cfg(test)]
mod tests_robust;
#[cfg(test)]
mod tests_rules;
#[cfg(test)]
mod tests_rules5;
#[cfg(test)]
mod tests_rules_fight;
#[cfg(test)]
mod tests_rules_skills;
#[cfg(test)]
mod tests_save_together;
#[cfg(test)]
mod tests_scripts;
#[cfg(test)]
mod tests_scripts2;
#[cfg(test)]
mod tests_spells;
#[cfg(test)]
mod tests_spells2;
#[cfg(test)]
mod tests_spells3;
#[cfg(test)]
mod tests_spells4;
#[cfg(test)]
mod tests_spells5;
#[cfg(test)]
mod tests_taming;
#[cfg(test)]
mod tests_terrain;
#[cfg(test)]
mod tests_world;
#[cfg(test)]
mod tests_world5;
pub mod transports;
pub mod update;
pub mod weather;
pub mod wind_stones;

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::time::Duration;

use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::game::data::GameData;
use crate::game::map::{MapEvent, Maps};
use crate::game::registry::{NameEntry, Registry};
use crate::game::store::{Startup, Store};

/// The settings maps and sessions read, from the config. The realm holds
/// it behind an atomically swapped pointer ([`Realm::config`]), so a
/// reload puts in a new one; a map takes it once an update ([`map::Map::cfg`]).
#[derive(Debug, Clone)]
pub struct RealmConfig {
    pub visibility_yards: f32,
    /// In dungeons and raids.
    pub instance_visibility_yards: f32,
    pub battleground_visibility_yards: f32,
    /// On a flight master's mount.
    pub flight_visibility_yards: f32,
    pub save_interval: Duration,
    pub motd: String,
    pub everyone_gm: bool,
    pub max_characters_per_account: usize,
    pub nav_dir: Option<std::path::PathBuf>,
    /// Thistle Tea's terrain tiles (`MMMMRRCC.map`): liquid kinds and areas.
    pub terrain_dir: Option<std::path::PathBuf>,
    /// How long a tile no one is near is kept (`[world] tile_unload_secs`);
    /// zero keeps them all.
    pub tile_unload: Duration,
    pub petition_signatures: u32,
    pub mail_delay: Duration,
    /// How long an empty copy stays up (`[instances] unload_delay_secs`).
    pub instance_unload: Duration,
    /// How long a logout outside an inn or flight takes (tests shorten it).
    pub logout_delay: Duration,
    /// Players a side needs for a match, over the template's.
    pub bg_min_players: Option<u32>,
    pub bg_start_delay: Duration,
    /// How long after a match ends everyone is sent home.
    pub bg_auto_leave: Duration,
    /// How long an invitation to a match stands.
    pub bg_invite_for: Duration,
    pub pvp_realm: bool,
    /// Packets a client may send a second, sustained.
    pub max_packets_per_sec: u32,
    /// Read-only queries a client may send a second, sustained, apart
    /// from the packets (`session::is_query`).
    pub max_queries_per_sec: u32,
    /// How seldom a `/who`, whois or inspect may come (`session::Cooldowns`):
    /// mangos's 5 s; none in the tests.
    pub packet_cooldown: Duration,
    /// The weekday honor's week turns over on: 1 Monday to 7 Sunday.
    pub honor_reset_weekday: u8,
    /// Whether spells roll to hit, crit and resist (tests fix them).
    pub spell_rolls: bool,
    /// Whether bolts and missiles take time to arrive (tests land them at
    /// once).
    pub spell_travel: bool,
    /// Whether the navmesh's and the terrain's tiles are read off the
    /// maps' threads (`map/tiles.rs`); tests read them at once, as they
    /// are asked for, so that what a test sees doesn't hang on the disk.
    pub tiles_in_background: bool,
    /// A match's leaver is a Deserter.
    pub bg_cast_deserter: bool,
    /// Honorable kills a week needs to be ranked.
    pub min_honor_kills: u32,
    /// The most a `/who` lists.
    pub max_who_list_returns: usize,
    /// Whether the zones' weather changes.
    pub weather: bool,
    pub weather_change: Duration,
    /// The least deposit an auction takes, in copper.
    pub auction_deposit_min: u32,
    pub arena: crate::game::map::arena::ArenaConfig,
    pub chat: crate::config::ChatConfig,
    pub rates: crate::config::RatesConfig,
    pub creatures: crate::config::CreaturesConfig,
    pub guards: crate::config::GuardsConfig,
    pub players: crate::config::PlayersConfig,
    pub environment: crate::config::EnvironmentConfig,
    pub quests: crate::config::QuestsConfig,
    pub instances: crate::config::InstancesConfig,
    pub outdoor_pvp: crate::config::OutdoorPvpConfig,
    pub events: crate::config::EventsConfig,
    pub anticheat: crate::config::AnticheatConfig,
    pub gm: crate::config::GmConfig,
}

impl RealmConfig {
    /// From a config checked at load (`crate::config::resolve`).
    pub fn from_config(c: &crate::config::ServerConfig) -> Self {
        let b = &c.battleground;
        Self {
            visibility_yards: c.visibility.continents_yards,
            instance_visibility_yards: c.visibility.instances_yards,
            battleground_visibility_yards: c.visibility.battleground_yards,
            flight_visibility_yards: c.visibility.flight_yards,
            save_interval: Duration::from_secs(c.world.save_interval_secs.max(10)),
            motd: c.world.motd.clone(),
            everyone_gm: c.accounts.everyone_gm,
            max_characters_per_account: c.players.characters_per_realm,
            nav_dir: c.world.nav_dir.clone(),
            terrain_dir: c.world.terrain_dir.clone(),
            tile_unload: Duration::from_secs(c.world.tile_unload_secs),
            petition_signatures: c.world.petition_signatures,
            mail_delay: Duration::from_secs(c.world.mail_delay_secs),
            instance_unload: Duration::from_secs(c.instances.unload_delay_secs),
            logout_delay: crate::game::map::login::LOGOUT_DELAY,
            bg_min_players: (b.min_players > 0).then_some(b.min_players),
            bg_start_delay: Duration::from_secs(b.start_delay_secs),
            bg_auto_leave: Duration::from_secs(b.auto_leave_secs),
            bg_invite_for: Duration::from_secs(b.invitation_secs),
            pvp_realm: c.players.pvp_realm,
            max_packets_per_sec: c.network.max_packets_per_sec,
            max_queries_per_sec: c.network.max_queries_per_sec,
            packet_cooldown: session::COOLDOWN,
            honor_reset_weekday: c.world.honor_reset_weekday.clamp(1, 7),
            spell_rolls: true,
            spell_travel: true,
            tiles_in_background: true,
            bg_cast_deserter: b.cast_deserter,
            min_honor_kills: c.world.min_honor_kills,
            max_who_list_returns: c.world.max_who_list_returns,
            weather: c.world.weather,
            weather_change: Duration::from_secs(c.world.weather_change_secs),
            auction_deposit_min: c.world.auction_deposit_min,
            arena: c.arena.clone(),
            chat: c.chat.clone(),
            rates: c.rates.clone(),
            creatures: c.creatures.clone(),
            guards: c.guards.clone(),
            players: c.players.clone(),
            environment: c.environment.clone(),
            quests: c.quests.clone(),
            instances: c.instances.clone(),
            outdoor_pvp: c.outdoor_pvp.clone(),
            events: c.events.clone(),
            anticheat: c.anticheat.clone(),
            gm: c.gm.clone(),
        }
    }

    /// This config after a reload to `applied` (whose keys that need a
    /// restart are as they were): what the config doesn't set kept.
    pub fn reloaded(&self, applied: &crate::config::ServerConfig) -> Self {
        Self {
            logout_delay: self.logout_delay,
            spell_rolls: self.spell_rolls,
            spell_travel: self.spell_travel,
            ..Self::from_config(applied)
        }
    }
}

impl Default for RealmConfig {
    fn default() -> Self {
        Self::from_config(&crate::config::ServerConfig::default())
    }
}

/// What a reload changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReloadReport {
    /// Keys whose new values are in, as `[section] key`.
    pub applied: Vec<String>,
    /// Keys changed in the file that wait for a restart.
    pub restart: Vec<String>,
    /// Values out of range, and what was used instead.
    pub warnings: Vec<String>,
}

impl std::fmt::Display for ReloadReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.applied.is_empty() {
            write!(f, "Config reloaded: nothing changed that a reload changes.")?;
        } else {
            write!(f, "Config reloaded: {}.", self.applied.join(", "))?;
        }
        if !self.restart.is_empty() {
            write!(f, " Needs a restart: {}.", self.restart.join(", "))?;
        }
        for w in &self.warnings {
            write!(f, " {w}.")?;
        }
        Ok(())
    }
}

/// Read `config.toml` and the environment again, off the map's threads,
/// and put in what is safe to change live.
pub async fn reload_from_disk(realm: Arc<Realm>) -> Result<ReloadReport, String> {
    reload_from(realm, std::path::PathBuf::from(crate::config::PATH)).await
}

/// [`reload_from_disk`], from `path`. A file missing or empty, where the
/// running settings didn't come from the defaults, is an error that
/// changes nothing: at a reload it is a file gone or half saved.
pub async fn reload_from(
    realm: Arc<Realm>,
    path: std::path::PathBuf,
) -> Result<ReloadReport, String> {
    let origin = path.display().to_string();
    let (loaded, blank) =
        tokio::task::spawn_blocking(move || crate::config::load_noting_blank(&path))
            .await
            .map_err(|e| format!("the reload stopped: {e}"))??;
    let mut source = realm.source.lock();
    if blank && loaded.config != *source {
        return Err(format!(
            "{origin} is missing or empty, and the running settings aren't the defaults: \
             nothing changed (put the file back, or give the defaults in it)"
        ));
    }
    Ok(realm.reload_locked(&mut source, loaded))
}

/// How long the shutdown waits for the maps to save their players.
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(15);

/// What every map and session shares.
pub struct Realm {
    /// The live settings; read them through [`Realm::config`], or in a map
    /// through `Map::cfg`.
    config: parking_lot::RwLock<Arc<RealmConfig>>,
    /// Bumped at each reload, after the settings are in: a map's update
    /// looks at it (one atomic load) and takes the lock only when it moved.
    config_generation: std::sync::atomic::AtomicU64,
    /// The config file's settings the live ones came from, for a reload's
    /// comparison.
    source: parking_lot::Mutex<crate::config::ServerConfig>,
    pub store: Store,
    pub data: Arc<GameData>,
    pub registry: Registry,
    pub maps: Maps,
    pub social: social::Social,
    /// The next character and item GUIDs: kept by the store with its
    /// writes, so none is given twice (`Store::keep_counters`).
    pub next_character: Arc<AtomicU32>,
    pub next_item: Arc<AtomicU32>,
    /// Bones ids, and the kept bones of maps yet to start.
    pub bones: map::bones::Kept,
    /// The auth server's sessions, for the world handshake.
    pub users: Option<crate::auth::UserCache>,
}

impl Realm {
    pub fn new(
        config: RealmConfig,
        store: Store,
        startup: Startup,
        data: GameData,
        users: Option<crate::auth::UserCache>,
    ) -> Arc<Self> {
        let registry = Registry::new();
        let social = social::Social::load(&startup.records);
        {
            let mut honor = social.honor.lock();
            honor.set_weekday(config.honor_reset_weekday);
            honor.minimum_kills = Some(config.min_honor_kills);
        }
        let max_mail_item = social
            .mail
            .lock()
            .max_item()
            .max(social.auctions.lock().max_item());
        for n in startup.names {
            registry.add_name(NameEntry {
                guid: guid::player(n.guid),
                account: Arc::from(n.account.as_str()),
                name: Arc::from(n.name.as_str()),
                race: n.race,
                class: n.class,
                gender: n.gender,
                level: n.level.into(),
            });
        }
        let next_character = Arc::new(AtomicU32::new(startup.max_character_guid + 1));
        let next_item = Arc::new(AtomicU32::new(startup.max_item_guid.max(max_mail_item) + 1));
        store.keep_counters(next_character.clone(), next_item.clone());
        let realm = Arc::new(Self {
            config: parking_lot::RwLock::new(Arc::new(config)),
            config_generation: std::sync::atomic::AtomicU64::new(0),
            source: parking_lot::Mutex::new(crate::config::ServerConfig::default()),
            store,
            data: Arc::new(data),
            registry,
            maps: Maps::default(),
            social,
            next_character,
            next_item,
            bones: map::bones::Kept::load(startup.bones, startup.max_bones_id),
            users,
        });
        *realm.social.events.lock() = events::active(&realm.data, chrono::Utc::now().timestamp());
        social::start_timers(&realm);
        transports::start(&realm);
        realm
    }

    /// The live settings (a read lock and an `Arc` clone: a map takes them
    /// once an update instead, when [`Realm::config_generation`] moved).
    pub fn config(&self) -> Arc<RealmConfig> {
        self.config.read().clone()
    }

    /// How many reloads there have been: read before [`Realm::config`],
    /// a map's settings are never older than the generation it keeps.
    pub fn config_generation(&self) -> u64 {
        self.config_generation
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// The config file's settings the live ones came from.
    pub fn set_source(&self, source: crate::config::ServerConfig) {
        *self.source.lock() = source;
    }

    /// Put in `loaded`'s values for what is safe to change live (rates,
    /// distances, timers, the MOTD); say which keys changed, and which of
    /// them need a restart. Maps see it at their next update.
    pub fn reload(&self, loaded: crate::config::Loaded) -> ReloadReport {
        let mut source = self.source.lock();
        self.reload_locked(&mut source, loaded)
    }

    /// [`Realm::reload`], `source` held.
    fn reload_locked(
        &self,
        source: &mut crate::config::ServerConfig,
        loaded: crate::config::Loaded,
    ) -> ReloadReport {
        let changes = crate::config::changes(source, &loaded.config);
        let applied = crate::config::live_part(source, &loaded.config);
        let next = self.config().reloaded(&applied);
        self.social.honor.lock().minimum_kills = Some(next.min_honor_kills);
        *self.config.write() = Arc::new(next);
        self.config_generation
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        *source = applied;
        let mut report = ReloadReport {
            warnings: loaded.warnings,
            ..Default::default()
        };
        for (key, reload) in changes {
            match reload {
                crate::config::Reload::Live => report.applied.push(key),
                crate::config::Reload::Restart => report.restart.push(key),
            }
        }
        info!("{report}");
        report
    }

    /// Save everyone in every map, and wait for the store to write it.
    pub async fn shutdown(&self) {
        self.shutdown_within(SHUTDOWN_WAIT).await;
    }

    /// [`Realm::shutdown`], waiting at most `limit` for the maps and those
    /// between them, and as long again for the store: a hung map is left
    /// behind, and what the others saved is still written.
    pub async fn shutdown_within(&self, limit: Duration) {
        let mut waits = Vec::new();
        for map in self.maps.close() {
            let (tx, rx) = tokio::sync::oneshot::channel();
            if map.send(MapEvent::Shutdown(tx)) {
                waits.push((map.key, rx));
            }
        }
        let deadline = tokio::time::Instant::now() + limit;
        for (key, w) in waits {
            if tokio::time::timeout_at(deadline, w).await.is_err() {
                tracing::error!(
                    "{key} didn't save its players within {} s (hung?); shutting down without them",
                    limit.as_secs()
                );
            }
        }
        // One between maps as it began (taken from one, their session yet
        // to hand them to the next) is saved by the map they go to, which
        // does so for whoever comes now (`Maps::get`, `map::finish`), and
        // goes offline once saved: waited for, within the same limit, so
        // that their save is written with the rest.
        while self.registry.online_count() > 0 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let left = self.registry.online_count();
        if left > 0 {
            tracing::error!(
                "{left} still online after {} s (between maps, or in a hung map); shutting down without them",
                limit.as_secs()
            );
        }
        // Honor earned since the last sweep (a kill, a battleground's win)
        // is kept too: the sweep that writes it runs only every so often.
        let honor = self.social.honor.lock().take_dirty();
        for e in honor {
            self.store
                .put_record(crate::game::honor::KIND, u64::from(e.guid), &e);
        }
        if tokio::time::timeout(limit, self.store.flush())
            .await
            .is_err()
        {
            tracing::error!(
                "the character store didn't finish writing within {} s",
                limit.as_secs()
            );
        }
    }
}

/// The world's data for a start over a store with `characters`, or why
/// the server mustn't start on it. Data looked for (`[world] data_dir`, or
/// `data/db` there) must load whole; with none looked for, the empty world
/// is only for a store with no characters (a bare checkout). A character
/// who logs in on a world without its data is saved without what the data
/// gives them, their skills first, for good.
pub fn world_data(dir: Option<&std::path::Path>, characters: usize) -> Result<GameData, String> {
    match dir {
        Some(dir) => data::load::load_checked(dir).map_err(|e| {
            format!(
                "the world's data won't load: {e}. Not starting: a character who logged in \
                 would be saved without their skills. Fix it, or point [world] data_dir at \
                 the data"
            )
        }),
        None if characters > 0 => Err(format!(
            "no world data (no {} and no [world] data_dir), and the store has {characters} \
             characters. Not starting: a character who logged in would be saved without \
             their skills. Put vmangos.sqlite and dbc.sqlite in {0}, or set [world] data_dir",
            crate::config::DEFAULT_DATA_DIR
        )),
        None => {
            warn!(
                "no world data: the world is empty. Put vmangos.sqlite and dbc.sqlite in {}, or set [world] data_dir",
                crate::config::DEFAULT_DATA_DIR
            );
            Ok(GameData::empty())
        }
    }
}

/// Where the world listens.
pub const WORLD_ADDRESS: &str = "0.0.0.0:8085";

/// Run the world server on `listener` (bound to [`WORLD_ADDRESS`] before
/// the store was touched) until a shutdown signal, then save everyone and
/// return the process's exit code: 2 if it can't start.
pub async fn run(users: crate::auth::UserCache, listener: TcpListener) -> u8 {
    let cfg = crate::config::config();
    let (store, startup) = Store::open(&cfg.world.characters_db).unwrap_or_else(|e| {
        panic!(
            "can't open the character store {}: {e}",
            cfg.world.characters_db.display()
        )
    });
    let data = match world_data(cfg.world.data_dir.as_deref(), startup.names.len()) {
        Ok(data) => data,
        Err(e) => {
            tracing::error!("{e}");
            return 2;
        }
    };
    let realm = Realm::new(
        RealmConfig::from_config(cfg),
        store,
        startup,
        data,
        Some(users),
    );
    realm.set_source(cfg.clone());
    reload_on_hangup(realm.clone());
    // The continents start now, so the first player in doesn't wait for
    // their spawns.
    for m in [0, 1] {
        realm.maps.get(&realm, map::MapKey::world(m));
    }
    if crate::tracy::on() {
        tokio::spawn(plot_realm(realm.clone()));
    }
    info!("wow_vanilla_server {}", crate::VERSION);
    info!("world: listening on {WORLD_ADDRESS}");
    let accept = {
        let realm = realm.clone();
        async move {
            let pending = Arc::new(tokio::sync::Semaphore::new(
                crate::config::config().network.max_pending_connections,
            ));
            loop {
                let (stream, peer) = match listener.accept().await {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("world: accept failed: {e}; retrying");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let Ok(permit) = pending.clone().try_acquire_owned() else {
                    tracing::debug!("world: too many connections in the handshake; closing {peer}");
                    continue;
                };
                let realm = realm.clone();
                // The permit covers the handshake only.
                tokio::spawn(session::serve(stream, realm, permit));
            }
        }
    };
    tokio::select! {
        _ = accept => {}
        _ = shutdown_signal() => {}
    }
    info!("shutting down: saving everyone");
    realm.shutdown().await;
    info!("saved; exiting");
    0
}

/// Tracy's plots of the realm, once a second while Tracy is on: who is
/// online, packets in and out a second, and the store's queue.
async fn plot_realm(realm: Arc<Realm>) {
    use crate::tracy::{Format, Plot};
    use std::sync::atomic::Ordering::Relaxed;
    let online = Plot::new("realm: players online", Format::Number);
    let packets_in = Plot::new("realm: packets in a second", Format::Number);
    let packets_out = Plot::new("realm: packets out a second", Format::Number);
    let store = Plot::new("realm: store queue", Format::Number);
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    let (mut last_in, mut last_out) = (
        conn::PACKETS_IN.load(Relaxed),
        conn::PACKETS_OUT.load(Relaxed),
    );
    loop {
        ticker.tick().await;
        let (now_in, now_out) = (
            conn::PACKETS_IN.load(Relaxed),
            conn::PACKETS_OUT.load(Relaxed),
        );
        online.set(realm.registry.online_count() as f64);
        packets_in.set(now_in.saturating_sub(last_in) as f64);
        packets_out.set(now_out.saturating_sub(last_out) as f64);
        store.set(realm.store.queued() as f64);
        (last_in, last_out) = (now_in, now_out);
    }
}

/// SIGHUP reloads the config, as `.reload config` does.
fn reload_on_hangup(realm: Arc<Realm>) {
    #[cfg(unix)]
    tokio::spawn(async move {
        let Ok(mut hup) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        else {
            warn!("no SIGHUP handler: reload the config with .reload config");
            return;
        };
        while hup.recv().await.is_some() {
            info!("SIGHUP: reloading the config");
            if let Err(e) = reload_from_disk(realm.clone()).await {
                warn!("the config wasn't reloaded: {e}");
            }
        }
    });
    #[cfg(not(unix))]
    let _ = realm;
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install the SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Milliseconds since the server started: the clock transports keep time
/// by.
pub fn uptime_ms() -> u32 {
    let ms = STARTED
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u32;
    #[cfg(test)]
    let ms = ms.wrapping_add(SKIPPED_MS.load(std::sync::atomic::Ordering::Relaxed));
    ms
}

#[cfg(test)]
static SKIPPED_MS: AtomicU32 = AtomicU32::new(0);

/// Move the transports' clock on `ms` (a test, waiting for a ship's
/// crossing minutes off). Each test is its own process under nextest.
#[cfg(test)]
pub fn skip_uptime(ms: u32) {
    SKIPPED_MS.fetch_add(ms, std::sync::atomic::Ordering::Relaxed);
}
