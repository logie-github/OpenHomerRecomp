import useBackend from '@openhome-core/backend/useBackend'
import { gen1RecompSyncPath } from '@openhome-core/save/gen1recomp/Gen1RecompSAV'
import { PathData } from '@openhome-core/save/util/path'
import { RemoteSave } from '@openhome-core/tauri/spectaCommands'
import { R } from '@openhome-core/util/functional'
import useDisplayError from '@openhome-ui/hooks/displayError'
import { Button, Card, Flex, Spinner, Text, TextField } from '@radix-ui/themes'
import { useCallback, useEffect, useState } from 'react'

interface Gen1RecompSyncProps {
  onOpen: (path: PathData) => Promise<void>
}

/**
 * Links OpenHome to a Gen1Recomp Save Sync account as one more device and
 * lists the account's playthroughs. Opening one loads it like any other save;
 * saving in OpenHome uploads it back to the account.
 */
export default function Gen1RecompSync({ onOpen }: Gen1RecompSyncProps) {
  const backend = useBackend()
  const displayError = useDisplayError()
  // undefined: not yet known; null: not linked
  const [linkedAs, setLinkedAs] = useState<string | null>()
  const [saves, setSaves] = useState<RemoteSave[]>()
  const [code1, setCode1] = useState('')
  const [code2, setCode2] = useState('')
  const [busy, setBusy] = useState(false)

  const refreshSaves = useCallback(async () => {
    setBusy(true)
    const result = await backend.gen1RecompSyncListSaves()
    setBusy(false)
    R.match(
      (list: RemoteSave[]) => setSaves(list),
      (err: string) => displayError('Error Reading Gen1Recomp Saves', err)
    )(result)
  }, [backend, displayError])

  useEffect(() => {
    if (linkedAs !== undefined) return
    backend.gen1RecompSyncStatus().then(
      R.match(
        (label) => {
          setLinkedAs(label)
          if (label !== null) refreshSaves()
        },
        (err) => {
          setLinkedAs(null)
          displayError('Error Reading Gen1Recomp Save Sync', err)
        }
      )
    )
  }, [backend, displayError, linkedAs, refreshSaves])

  const link = useCallback(async () => {
    setBusy(true)
    const result = await backend.gen1RecompSyncLink(code1, code2)
    setBusy(false)
    if (R.isErr(result)) {
      displayError('Error Linking Gen1Recomp Save Sync', result.error)
      return
    }
    setCode1('')
    setCode2('')
    setLinkedAs(undefined)
  }, [backend, code1, code2, displayError])

  const unlink = useCallback(async () => {
    setBusy(true)
    const result = await backend.gen1RecompSyncUnlink()
    setBusy(false)
    if (R.isErr(result)) {
      displayError('Error Unlinking Gen1Recomp Save Sync', result.error)
      return
    }
    setSaves(undefined)
    setLinkedAs(null)
  }, [backend, displayError])

  if (linkedAs === undefined) {
    return (
      <Flex justify="center" p="4">
        <Spinner />
      </Flex>
    )
  }

  if (linkedAs === null) {
    return (
      <Flex direction="column" gap="3" p="4" style={{ maxWidth: 480 }}>
        <Text>
          In Gen1Recomp, open <b>Save Sync</b> and create (or show) your sync account, then enter
          its two codes here. OpenHome joins the account as another device.
        </Text>
        <TextField.Root
          placeholder="Code 1 (0000-0000)"
          value={code1}
          onChange={(e) => setCode1(e.target.value)}
        />
        <TextField.Root
          placeholder="Code 2 (0000-0000)"
          value={code2}
          onChange={(e) => setCode2(e.target.value)}
        />
        <Button onClick={link} disabled={busy || !code1 || !code2}>
          {busy ? <Spinner /> : 'Link'}
        </Button>
      </Flex>
    )
  }

  return (
    <Flex direction="column" gap="2" p="2" height="100%">
      <Flex gap="2" justify="end" align="center">
        <Text size="1" color="gray" style={{ marginRight: 'auto' }}>
          Save in Gen1Recomp and let it sync before opening a playthrough here. After saving in
          OpenHome, let Gen1Recomp sync before you play on.
        </Text>
        <Button onClick={refreshSaves} disabled={busy} variant="soft">
          {busy ? <Spinner /> : 'Refresh'}
        </Button>
        <Button onClick={unlink} disabled={busy} color="red" variant="soft">
          Unlink
        </Button>
      </Flex>
      <Flex overflowY="auto" direction="column" gap="1">
        {saves?.length === 0 && <Text>No playthroughs on this sync account yet.</Text>}
        {saves?.map((save) => (
          <Card key={save.key}>
            <Flex direction="row" gap="4" align="center">
              <b>{save.trainerName ?? save.playthroughId}</b>
              <Text color="gray">{save.version.toUpperCase()}</Text>
              {save.badges !== null && <Text color="gray">{save.badges} badges</Text>}
              {save.timeText && <Text color="gray">{save.timeText}</Text>}
              <Button
                style={{ marginLeft: 'auto' }}
                disabled={!save.supported}
                title={save.supported ? undefined : 'Only Red, Blue and Yellow are supported'}
                onClick={() =>
                  onOpen(
                    gen1RecompSyncPath(
                      save.version,
                      save.playthroughId,
                      save.trainerName ?? save.playthroughId
                    )
                  )
                }
              >
                Open
              </Button>
            </Flex>
          </Card>
        ))}
      </Flex>
    </Flex>
  )
}
