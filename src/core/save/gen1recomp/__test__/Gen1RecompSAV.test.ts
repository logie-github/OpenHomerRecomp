import { PK1 } from '@openhome-core/pkm'
import { OHPKM } from '@openhome-core/pkm/OHPKM'
import { R } from '@openhome-core/util/functional'
import { ConvertStrategies, OriginGame } from '@pkm-rs/pkg'
import assert from 'assert'
import { beforeAll, describe, expect, test } from 'vitest'
import { initializeWasm } from '../../__test__/init'
import { buildUnknownSaveFile } from '../../util/load'
import { emptyPathData } from '../../util/path'
import { Gen1RecompSAV, gen1RecompSyncPath, parseGen1RecompSyncPath } from '../Gen1RecompSAV'
import { isLuaTable, luaArray, luaTable, LuaTable, parseLuaSave, serializeLuaSave } from '../lua'

beforeAll(initializeWasm)

// Shaped like Gen1Recomp's SaveSerializer output (sorted keys, trailing commas).
// Box 1 holds a nicknamed Pikachu, an un-nicknamed Nidoran♂ with a mod field,
// and a species OpenHome cannot read; box 2 holds a legacy-shaped mon with no
// stat block or typeBytes, as a .sav-imported box mon has.
const SAVE_LUA = `return {
  boxes = {
    [1] = {
      [1] = {
        catchRate = 190,
        dvs = {
          attack = 15,
          defense = 12,
          hp = 9,
          special = 9,
          speed = 10,
        },
        exp = 15625,
        hp = 40,
        level = 25,
        moves = {
          [1] = {
            id = "THUNDERSHOCK",
            pp = 30,
          },
          [2] = {
            id = "PSYCHIC_M",
            pp = 5,
            ppUps = 3,
          },
        },
        nickname = "SPARKY",
        ot = "ASH",
        otId = 12345,
        species = "PIKACHU",
        statExp = {
          attack = 20,
          defense = 30,
          hp = 10,
          special = 50,
          speed = 40,
        },
        status = "PAR",
      },
      [2] = {
        catchRate = 235,
        dvs = {
          attack = 1,
          defense = 2,
          hp = 10,
          special = 4,
          speed = 3,
        },
        exp = 1000,
        hp = 20,
        level = 10,
        modField = "kept \\"as is\\"\\n",
        moves = {
          [1] = {
            id = "TACKLE",
            pp = 35,
          },
        },
        ot = "GARY",
        otId = 777,
        species = "NIDORAN_M",
        statExp = {
          attack = 0,
          defense = 0,
          hp = 0,
          special = 0,
          speed = 0,
        },
        typeBytes = {
          [1] = 3,
          [2] = 3,
        },
      },
      [3] = {
        exp = 500,
        level = 9,
        species = "MISSINGMON",
      },
    },
    [2] = {
      [1] = {
        dvs = {
          attack = 8,
          defense = 8,
          hp = 0,
          special = 8,
          speed = 8,
        },
        exp = 135,
        hp = 19,
        level = 5,
        moves = {
          [1] = {
            id = "SCRATCH",
            pp = 35,
          },
        },
        otId = 12345,
        species = "CHARMANDER",
      },
    },
    [3] = {},
  },
  currentBox = 2,
  meta = {
    format = 5,
    playthroughId = "quiet-forest-dawn",
  },
  party = {},
  playTime = 7265.5,
  player = {
    id = 12345,
    name = "ASH",
  },
  version = "yellow",
}
`

const encoder = new TextEncoder()

function load(text = SAVE_LUA): Gen1RecompSAV {
  const result = buildUnknownSaveFile(emptyPathData, encoder.encode(text), [Gen1RecompSAV])
  if (R.isErr(result)) assert.fail(result.error)
  return result.data as Gen1RecompSAV
}

function boxRecords(save: Gen1RecompSAV, box: number): LuaTable[] {
  const root = parseLuaSave(new TextDecoder().decode(save.bytes))
  return luaArray(luaArray(luaTable(root, 'boxes'))[box] as LuaTable).filter(isLuaTable)
}

describe('lua', () => {
  test('serializes an untouched save back to the same text', () => {
    expect(serializeLuaSave(parseLuaSave(SAVE_LUA))).toEqual(SAVE_LUA)
  })

  test('reads escapes as upstream %q writes them', () => {
    const root = parseLuaSave('return {\n  a = "x\\\ny\\0z\\226\\153\\130",\n}\n')
    expect(root.get('a')).toEqual('x\ny\0z♂')
  })
})

describe('Gen1RecompSAV', () => {
  test('detects Generation I saves only', () => {
    expect(Gen1RecompSAV.fileIsSave(encoder.encode(SAVE_LUA))).toBe(true)
    expect(Gen1RecompSAV.fileIsSave(encoder.encode(SAVE_LUA.replace('"yellow"', '"gold"')))).toBe(
      false
    )
    expect(Gen1RecompSAV.fileIsSave(new Uint8Array(0x8000))).toBe(false)
  })

  test('reads trainer and boxes', () => {
    const save = load()
    expect(save.origin).toEqual(OriginGame.Yellow)
    expect(save.name).toEqual('ASH')
    expect(save.tid).toEqual(12345)
    expect(save.getBoxCount()).toEqual(3)
    expect(save.currentPCBox).toEqual(1)
  })

  test('maps Lua fields onto PK1', () => {
    const save = load()
    const pikachu = save.getMonAt(0, 0)
    assert(pikachu)
    expect(pikachu.nationalDex).toEqual(25)
    expect(pikachu.nickname).toEqual('SPARKY')
    expect(pikachu.trainerName).toEqual('ASH')
    expect(pikachu.trainerID).toEqual(12345)
    expect(pikachu.exp).toEqual(15625)
    expect(pikachu.level).toEqual(25)
    expect(pikachu.currentHP).toEqual(40)
    expect(pikachu.statusCondition).toEqual(1 << 6)
    expect(pikachu.dvs).toEqual({ atk: 15, def: 12, spe: 10, spc: 9, hp: 9 })
    expect(pikachu.evsG12).toEqual({ hp: 10, atk: 20, def: 30, spe: 40, spc: 50 })
    expect(pikachu.moves).toEqual([84, 94, 0, 0])
    expect(pikachu.movePP).toEqual([30, 5, 0, 0])
    expect(pikachu.movePPUps).toEqual([0, 3, 0, 0])
    expect(pikachu.gameOfOrigin).toEqual(OriginGame.Yellow)

    // not nicknamed: the cartridge's species name, as G1SAV would read it
    expect(save.getMonAt(0, 1)?.nickname).toEqual('NIDORAN♂')
    // no OT recorded: the player's, as Gen1Recomp's own exporter assumes
    expect(save.getMonAt(1, 0)?.trainerName).toEqual('ASH')
  })

  test('locks slots OpenHome cannot read and keeps them', () => {
    const save = load()
    expect(save.getMonAt(0, 2)).toBeUndefined()
    expect(save.getSlotMetadata(0, 2).isDisabled).toBe(true)

    save.setMonAt(0, 0, undefined)
    save.updatedBoxSlots.push({ box: 0, boxSlot: 0 })
    save.prepareForSaving()

    const records = boxRecords(save, 0)
    expect(records.map((r) => r.get('species'))).toEqual(['NIDORAN_M', 'MISSINGMON'])
  })

  test('writes untouched boxes and Pokémon back verbatim', () => {
    const save = load()
    save.updatedBoxSlots.push({ box: 0, boxSlot: 0 })
    save.prepareForSaving()
    expect(new TextDecoder().decode(save.bytes)).toEqual(SAVE_LUA)
  })

  test('moving a Pokémon out and back through OpenHome keeps it intact', () => {
    const save = load()
    const nidoran = save.getMonAt(0, 1)
    assert(nidoran)
    const ohpkm = OHPKM.fromMonInSave(nidoran, save)

    save.setMonAt(0, 1, undefined)
    save.updatedBoxSlots.push({ box: 0, boxSlot: 1 })
    save.prepareForSaving()
    expect(boxRecords(save, 0).map((r) => r.get('species'))).toEqual(['PIKACHU', 'MISSINGMON'])

    const reloaded = load(new TextDecoder().decode(save.bytes))
    const back = R.assert(reloaded.convertOhpkm(ohpkm, ConvertStrategies.getDefault()))
    reloaded.setMonAt(2, 0, back)
    reloaded.updatedBoxSlots.push({ box: 2, boxSlot: 0 })
    reloaded.prepareForSaving()

    const [record] = boxRecords(reloaded, 2)
    expect(record.get('species')).toEqual('NIDORAN_M')
    expect(record.has('nickname')).toBe(false)
    expect(record.get('ot')).toEqual('GARY')
    expect(record.get('otId')).toEqual(777)
    expect(record.get('exp')).toEqual(1000)
    expect(record.get('level')).toEqual(back.level)
    const dvs = luaTable(record, 'dvs')
    expect([
      dvs?.get('attack'),
      dvs?.get('defense'),
      dvs?.get('speed'),
      dvs?.get('special'),
    ]).toEqual([1, 2, 3, 4])
    expect(luaArray(luaTable(record, 'moves')).map((m) => (m as LuaTable).get('id'))).toEqual([
      'TACKLE',
    ])
    expect(luaTable(record, 'stats')?.get('hp')).toEqual(back.getStats().hp)

    const again = load(new TextDecoder().decode(reloaded.bytes)).getMonAt(2, 0)
    assert(again)
    expect(again.nickname).toEqual(nidoran.nickname)
    expect(again.trainerID).toEqual(nidoran.trainerID)
    expect(again.dvs).toEqual(nidoran.dvs)
  })

  test('a Pokémon from another game gets a complete record', () => {
    const save = load()
    const pikachu = save.getMonAt(0, 0)
    assert(pikachu)
    const fresh = R.assert(
      PK1.fromOhpkm(OHPKM.fromMonUnknownSave(pikachu), ConvertStrategies.getDefault())
    )
    save.setMonAt(1, 5, fresh)
    save.updatedBoxSlots.push({ box: 1, boxSlot: 5 })
    save.prepareForSaving()
    const records = boxRecords(save, 1)
    expect(records.map((r) => r.get('species'))).toEqual(['CHARMANDER', 'PIKACHU'])
    const record = records[1]
    for (const key of ['level', 'exp', 'hp', 'dvs', 'statExp', 'stats', 'moves', 'otId', 'ot']) {
      expect(record.has(key)).toBe(true)
    }
    expect(record.has('modField')).toBe(false)
  })
})

describe('sync paths', () => {
  test('round trip', () => {
    const path = gen1RecompSyncPath('red', 'quiet-forest-dawn', 'ASH')
    expect(path.raw).toEqual('gen1recomp-sync://red/quiet-forest-dawn')
    expect(parseGen1RecompSyncPath(path.raw)).toEqual({
      version: 'red',
      playthroughId: 'quiet-forest-dawn',
    })
    expect(parseGen1RecompSyncPath('/home/me/red.sav')).toBeUndefined()
  })
})
