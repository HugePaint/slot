use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use slot_store::Cart;
use slot_ui::{board_face, cart_face, padded, CartFace, TURN_PAD};

pub struct BuiltFaces {
    pub stem: String,
    pub board: CartFace,
    pub lid: CartFace,
}

pub struct FaceBuilder {
    requests: Sender<Cart>,
    built: Receiver<BuiltFaces>,
}

impl FaceBuilder {
    pub fn spawn() -> Self {
        let (requests, inbox) = mpsc::channel::<Cart>();
        let (outbox, built) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("slot-faces".into())
            .spawn(move || {
                while let Ok(mut cart) = inbox.recv() {
                    while let Ok(newer) = inbox.try_recv() {
                        cart = newer;
                    }
                    let faces = BuiltFaces {
                        stem: cart.stem.clone(),
                        board: board_face(&cart),
                        lid: padded(&cart_face(&cart), TURN_PAD),
                    };
                    if outbox.send(faces).is_err() {
                        return;
                    }
                }
            });
        if let Err(e) = spawned {
            eprintln!("slot: faces: worker thread failed to start: {e}");
        }
        FaceBuilder { requests, built }
    }

    pub fn request(&self, cart: Cart) {
        let _ = self.requests.send(cart);
    }

    pub fn take(&self) -> Option<BuiltFaces> {
        let mut newest = None;
        while let Ok(faces) = self.built.try_recv() {
            newest = Some(faces);
        }
        newest
    }
}

/// The shelf's faces, rasterised off the frame loop one cart at a time.
///
/// A cart face costs about 70 ms here, measured, and the shelf draws three carts either side
/// of the caret. Rasterising the whole card before the first frame is therefore seven seconds
/// on a hundred games — all of it spent on carts nobody is looking at yet. The shelf is born
/// with the seven it draws and this fills in the rest while the user reads them, nearest the
/// caret first, so a card of any size opens in the time seven faces take.
///
/// One request in flight at a time: the answer is always wanted, and a queue would only let
/// the worker fall further behind the caret.
pub struct ShelfFaceFiller {
    requests: Sender<(usize, Cart)>,
    built: Receiver<(usize, CartFace)>,
    outstanding: Option<usize>,
}

impl ShelfFaceFiller {
    pub fn spawn() -> Self {
        let (requests, inbox) = mpsc::channel::<(usize, Cart)>();
        let (outbox, built) = mpsc::channel();
        let spawned = thread::Builder::new()
            .name("slot-shelf-faces".into())
            .spawn(move || {
                while let Ok((i, cart)) = inbox.recv() {
                    if outbox.send((i, cart_face(&cart))).is_err() {
                        return;
                    }
                }
            });
        if let Err(e) = spawned {
            eprintln!("slot: shelf faces: worker thread failed to start: {e}");
        }
        ShelfFaceFiller {
            requests,
            built,
            outstanding: None,
        }
    }

    pub fn busy(&self) -> bool {
        self.outstanding.is_some()
    }

    /// The index being built, if any. Used to keep the request weighted toward the caret.
    pub fn outstanding(&self) -> Option<usize> {
        self.outstanding
    }

    /// Ask for one. Refused while another is in flight.
    pub fn request(&mut self, i: usize, cart: Cart) -> bool {
        if self.outstanding.is_some() {
            return false;
        }
        if self.requests.send((i, cart)).is_err() {
            return false;
        }
        self.outstanding = Some(i);
        true
    }

    /// Everything that finished since the last call, in completion order.
    pub fn drain(&mut self) -> Vec<(usize, CartFace)> {
        let mut out = Vec::new();
        while let Ok(done) = self.built.try_recv() {
            self.outstanding = None;
            out.push(done);
        }
        out
    }
}

/// How many carts either side of the caret the shelf can draw at once. `Shelf::SLOTS` in
/// `slot-ui`; duplicated rather than exported because it is a drawing decision there and a
/// boot-budget decision here, and the two are allowed to disagree.
pub const SHELF_RADIUS: usize = 3;

/// The indices the shelf can currently show, as a contiguous window centred on `index` and
/// clipped to the ends of the list. Fewer than `SHELF_RADIUS * 2 + 1` only when there are
/// fewer carts than that.
pub fn shelf_window(index: usize, count: usize) -> Vec<usize> {
    if count == 0 {
        return Vec::new();
    }
    let span = (SHELF_RADIUS * 2 + 1).min(count);
    let start = index.saturating_sub(SHELF_RADIUS).min(count - span);
    (start..start + span).collect()
}

/// Stops `boot.log` from reporting a number nobody can act on: `28.1` ms is the caret's own
/// face, which is built ahead of the row.
pub fn ring_distance(a: usize, b: usize, count: usize) -> usize {
    let d = a.abs_diff(b);
    d.min(count.saturating_sub(d))
}
