# Authoritative netcode reconciliation plan

Status: **final design recorded; not yet implemented**.

This is the plan for authoritative client/server replication with client-side
prediction and reconciliation. The server is the source of truth; the client
predicts from its own inputs and reconciles on each authoritative tick. Real
information always beats extrapolation. A real desync is corrected with an
authoritative checkpoint instead of an unconditionally huge raw-world transfer.

## Final design

```text
// ========================================
// SERVER
// ========================================

state:
    A          // authoritative deterministic world
    TA         // current authoritative tick
    BA         // real input buffer, sorted by (tick, player, sequence)
    history    // recent canonical inputs/checkpoints

onRealInput(input):
    BA.insert(input)

serverTick():
    inputs = BA.realInputsFor(TA)
    inputs = canonicalSort(inputs)

    A.simulateTick(inputs)

    h = hash(A)

    broadcastCanonical({
        tick: TA,
        inputs: inputs,
        hash: h
    })

    history.store(TA, inputs, optionalCheckpoint(A))
    BA.removeFinalized(TA)

    TA++


// Server intentionally runs D ticks behind real time,
// giving normal network traffic time to reach BA.


// ========================================
// CLIENT
// ========================================

state:
    A          // confirmed world
    TA         // latest confirmed tick

    BS         // speculative input buffer
               // entries can be:
               //   REAL_LOCAL
               //   REAL_REMOTE
               //   EXTRAPOLATED

    S          // speculative simulation world
    V          // presentation/render state


// ----------------------------------------
// Local input
// ----------------------------------------

onLocalInput(realInput):
    BS.insert(realInput, REAL_LOCAL)

    sendToServer(realInput)
    sendToPeers(realInput)

    rebuildSIfNecessary()


// ----------------------------------------
// Input received directly from another player
// ----------------------------------------

onPeerInput(realInput):
    old = BS[input.key]

    // Real information always beats extrapolation.
    BS.replace(realInput, REAL_REMOTE)

    if old was EXTRAPOLATED or old != realInput:
        rebuildSFromAffectedTick()


// ----------------------------------------
// Speculation
// ----------------------------------------

buildSpeculativeWorld():
    S = clone(A)

    for tick in TA+1 .. currentRealtimeTick:
        inputs = BS.realInputsFor(tick)

        for player missing input at tick:
            predicted = extrapolateInput(player, tick)
            BS.insert(predicted, EXTRAPOLATED)
            inputs.add(predicted)

        inputs = deterministicSort(inputs)

        S.simulateTick(inputs)


// ----------------------------------------
// Authoritative update from server
// ----------------------------------------

onCanonicalTick(packet):
    // packet = { tick, canonicalInputs, hash }

    canonical = packet.inputs

    A.simulateTick(canonical)
    TA = packet.tick

    if hash(A) != packet.hash:
        requestAuthoritativeCheckpoint()
        return

    // These ticks are no longer speculative.
    BS.removeConfirmedOrRejectedInputsThrough(TA)

    // A changed, therefore our predicted future is rebuilt.
    buildSpeculativeWorld()


// ----------------------------------------
// Desync recovery
// ----------------------------------------

onAuthoritativeCheckpoint(checkpoint):
    A = checkpoint.world
    TA = checkpoint.tick

    discardInvalidSpeculation()
    buildSpeculativeWorld()


// ----------------------------------------
// Rendering
// ----------------------------------------

render(dt):
    // Never modify deterministic S for visual smoothing.
    // V is purely presentation state.

    for entity:
        V[entity].position =
            smooth(V[entity].position, S[entity].position, dt)

        V[entity].rotation =
            smooth(V[entity].rotation, S[entity].rotation, dt)

    draw(V)
```

So the four important pieces are **`A` = confirmed truth, `BS` = real +
extrapolated unconfirmed inputs, `S` = predicted deterministic world,
`V` = visually smoothed representation of `S`**.
