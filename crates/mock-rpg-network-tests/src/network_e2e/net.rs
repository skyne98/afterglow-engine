use super::model::ClientInput;

#[derive(Default)]
pub(super) struct MockNetwork {
    packets: Vec<Packet>,
    order: u64,
}

#[derive(Clone)]
struct Packet {
    deliver_at: u32,
    order: u64,
    input: ClientInput,
}

impl MockNetwork {
    pub(super) fn send(&mut self, input: ClientInput, latency_ticks: u32) {
        self.packets.push(Packet {
            deliver_at: input.tick.saturating_add(latency_ticks),
            order: self.order,
            input,
        });
        self.order = self.order.saturating_add(1);
    }

    pub(super) fn deliver(&mut self, server_tick: u32) -> Vec<ClientInput> {
        let mut ready = Vec::new();
        let mut pending = Vec::new();
        for packet in self.packets.drain(..) {
            if packet.deliver_at <= server_tick {
                ready.push(packet);
            } else {
                pending.push(packet);
            }
        }
        self.packets = pending;
        ready.sort_by_key(|packet| (packet.deliver_at, packet.order));
        ready.into_iter().map(|packet| packet.input).collect()
    }
}
