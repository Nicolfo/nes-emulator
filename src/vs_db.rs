//! Vs. System game database: hardware details iNES 1.0 headers can't
//! express, keyed by the CRC32 of PRG+CHR ROM. Extracted from Mesen2's
//! MesenNesDB.txt (GPL-3.0; itself built from NewRisingSun's NES 2.0 header
//! database, NesCartDB and Nestopia's DB).

/// (CRC32, Vs. hardware type, PPU model, input type) - Mesen's encodings:
/// hardware = NES 2.0 byte 13 high nibble; PPU 0-1 = 2C03, 2-5 = 2C04-0001..4,
/// 6-10 = 2C05-01..05; input = NES 2.0 expansion device ($04-$07).
pub(crate) const VS_GAMES: [(u32, u8, u8, u8); 76] = [
    (0x01357944, 0, 2, 5),  // mapper 99, 32 KB PRG
    (0x071CC00B, 5, 0, 4),  // mapper 99, 64 KB PRG
    (0x07A2F3B2, 0, 10, 7), // mapper 99, 40 KB PRG
    (0x0FA322C2, 0, 5, 5),  // mapper 99, 32 KB PRG
    (0x12012CD9, 3, 2, 4),  // mapper 206, 128 KB PRG
    (0x159EF3C1, 0, 0, 4),  // mapper 99, 32 KB PRG
    (0x16AA4E2D, 0, 2, 7),  // mapper 99, 32 KB PRG
    (0x1A0E5D4C, 5, 4, 4),  // mapper 99, 64 KB PRG
    (0x1CA45A6D, 4, 5, 5),  // mapper 99, 32 KB PRG
    (0x2019FE65, 0, 2, 4),  // mapper 67, 128 KB PRG
    (0x21A653C7, 0, 2, 5),  // mapper 206, 32 KB PRG
    (0x22BD8A28, 5, 2, 4),  // mapper 99, 64 KB PRG
    (0x2B85420E, 0, 4, 5),  // mapper 1, 64 KB PRG
    (0x2BF21F5F, 5, 3, 4),  // mapper 99, 64 KB PRG
    (0x2C1CC6DA, 5, 4, 4),  // mapper 99, 64 KB PRG
    (0x30586A4D, 5, 5, 5),  // mapper 99, 64 KB PRG
    (0x38C1D042, 5, 2, 4),  // mapper 99, 64 KB PRG
    (0x40EF00FE, 0, 5, 4),  // mapper 99, 32 KB PRG
    (0x412E5A0D, 0, 10, 7), // mapper 99, 48 KB PRG
    (0x48361E79, 0, 3, 5),  // mapper 99, 32 KB PRG
    (0x51C76943, 5, 0, 4),  // mapper 99, 48 KB PRG
    (0x52C501D0, 2, 4, 4),  // mapper 206, 64 KB PRG
    (0x535E6152, 0, 9, 4),  // mapper 99, 32 KB PRG
    (0x57685E6C, 5, 2, 4),  // mapper 99, 64 KB PRG
    (0x5FD6F603, 5, 5, 5),  // mapper 99, 64 KB PRG
    (0x63ABF889, 0, 3, 5),  // mapper 99, 32 KB PRG
    (0x66471EFE, 0, 0, 6),  // mapper 99, 32 KB PRG
    (0x67BAC152, 5, 0, 4),  // mapper 99, 64 KB PRG
    (0x6B8E7614, 5, 2, 4),  // mapper 99, 64 KB PRG
    (0x70FFB591, 6, 3, 5),  // mapper 99, 40 KB PRG
    (0x74F713B4, 0, 4, 4),  // mapper 75, 64 KB PRG
    (0x766C2CAC, 0, 4, 5),  // mapper 99, 32 KB PRG
    (0x792CF17F, 5, 0, 4),  // mapper 99, 64 KB PRG
    (0x7CFF0F84, 0, 5, 4),  // mapper 99, 32 KB PRG
    (0x832CF592, 0, 2, 7),  // mapper 206, 128 KB PRG
    (0x8337E123, 1, 2, 5),  // mapper 206, 64 KB PRG
    (0x866F145C, 2, 4, 0),  // mapper 206, 128 KB PRG
    (0x889A02A9, 0, 3, 5),  // mapper 99, 32 KB PRG
    (0x89A2FD37, 0, 4, 5),  // mapper 99, 32 KB PRG
    (0x89B56859, 0, 0, 0),  // mapper 206, 128 KB PRG
    (0x8C0C2DF5, 0, 11, 4), // mapper 2, 128 KB PRG
    (0x9768E5E0, 0, 6, 5),  // mapper 99, 32 KB PRG
    (0x98E3C75A, 0, 5, 4),  // mapper 99, 32 KB PRG
    (0x994A2711, 5, 0, 4),  // mapper 99, 48 KB PRG
    (0x9AE2BAA0, 0, 3, 4),  // mapper 99, 32 KB PRG
    (0xA21330FE, 5, 2, 4),  // mapper 99, 64 KB PRG
    (0xA3D60939, 5, 2, 4),  // mapper 99, 64 KB PRG
    (0xABE1A0C2, 0, 0, 7),  // mapper 99, 32 KB PRG
    (0xABE8E174, 0, 2, 4),  // mapper 99, 32 KB PRG
    (0xAECA9D42, 0, 2, 5),  // mapper 99, 32 KB PRG
    (0xB194CA80, 0, 0, 7),  // mapper 99, 48 KB PRG
    (0xB1C4C508, 0, 3, 5),  // mapper 99, 32 KB PRG
    (0xB6AFFFF8, 0, 2, 4),  // mapper 0, 32 KB PRG
    (0xB7C37203, 0, 0, 0),  // mapper 99, 32 KB PRG
    (0xC20E9CA7, 0, 8, 5),  // mapper 99, 32 KB PRG
    (0xC31845AB, 0, 2, 7),  // mapper 99, 32 KB PRG
    (0xC492B4D1, 0, 2, 5),  // mapper 99, 24 KB PRG
    (0xC95321A8, 0, 4, 4),  // mapper 99, 32 KB PRG
    (0xCA3E9B1A, 0, 2, 4),  // mapper 99, 32 KB PRG
    (0xCD059AEA, 5, 0, 4),  // mapper 99, 64 KB PRG
    (0xDF07C203, 0, 0, 0),  // mapper 206, 128 KB PRG
    (0xE26593E1, 0, 3, 5),  // mapper 99, 32 KB PRG
    (0xE45485A5, 1, 2, 5),  // mapper 206, 64 KB PRG
    (0xE8B20197, 0, 3, 4),  // mapper 99, 32 KB PRG
    (0xE9A6F17D, 0, 1, 5),  // mapper 99, 32 KB PRG
    (0xEE8AF512, 0, 2, 5),  // mapper 75, 64 KB PRG
    (0xEF7AF338, 0, 5, 5),  // mapper 99, 32 KB PRG
    (0xF06FC822, 0, 4, 4),  // mapper 99, 32 KB PRG
    (0xF6DE2AA2, 0, 0, 0),  // mapper 151, 64 KB PRG
    (0xF735D926, 0, 2, 5),  // mapper 151, 64 KB PRG
    (0xF9DB47BD, 6, 3, 5),  // mapper 99, 64 KB PRG
    (0xFB0DDDE7, 0, 5, 4),  // mapper 99, 32 KB PRG
    (0xFC8E252A, 5, 2, 4),  // mapper 99, 64 KB PRG
    (0xFCFB4236, 5, 2, 4),  // mapper 99, 64 KB PRG
    (0xFD770F82, 5, 3, 4),  // mapper 99, 64 KB PRG
    (0xFFBEF374, 0, 3, 4),  // mapper 2, 128 KB PRG
];
