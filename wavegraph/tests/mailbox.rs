//! Exercise buffer ownership in both directions across real host threads.
#![cfg(target_has_atomic = "ptr")]

use core::hint::spin_loop;
use wavegraph::mailbox::Mailbox;

#[test]
fn a_full_slot_returns_the_original_owner_and_survives_resplitting() {
    let mut samples = [0; 7];
    let mut other = [1; 7];
    let mut mailbox = Mailbox::new();
    {
        let (mut tx, mut rx) = mailbox.split();
        assert!(rx.try_recv().is_none());
        tx.try_send(&mut samples).expect("empty mailbox");
        let returned = tx.try_send(&mut other).expect_err("slot is occupied");
        returned[0] = 2;
    }
    let (_, mut rx) = mailbox.split();
    rx.try_recv().expect("published buffer")[0] = 3;
    drop(mailbox);
    assert_eq!(samples[0], 3);
    assert_eq!(other[0], 2);
}

#[test]
fn a_worker_keeps_its_render_state_and_returns_exclusive_samples() {
    let mut samples = [0u32; 23];
    let mut jobs = Mailbox::new();
    let mut done = Mailbox::new();
    std::thread::scope(|scope| {
        let (mut submit, mut receive_job) = jobs.split();
        let (mut complete, mut receive_block) = done.split();
        let worker = scope.spawn(move || {
            for generation in 1..=2_000 {
                let block: &mut [u32; 23] = loop {
                    if let Some(block) = receive_job.try_recv() {
                        break block;
                    }
                    spin_loop();
                };
                assert!(block.iter().all(|&sample| sample == generation - 1));
                block.fill(generation);
                complete.try_send(block).expect("previous block consumed");
            }
        });
        let mut block = &mut samples;
        for generation in 1..=2_000 {
            submit.try_send(block).expect("worker returned the slot");
            block = loop {
                if let Some(block) = receive_block.try_recv() {
                    break block;
                }
                spin_loop();
            };
            assert!(block.iter().all(|&sample| sample == generation));
        }
        worker.join().expect("worker finished");
    });
}
