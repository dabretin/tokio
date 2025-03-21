use crate::loom::sync::atomic::AtomicUsize;
use crate::loom::sync::Arc;
use crate::loom::thread;
use crate::runtime::{Builder, Runtime};
use crate::sync::oneshot::{self, Receiver};
use crate::task;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::Ordering::{Acquire, Release};
use std::task::{Context, Poll};

fn assert_at_most_num_polls(rt: Arc<Runtime>, at_most_polls: usize) {
    let (tx, rx) = oneshot::channel();
    let num_polls = Arc::new(AtomicUsize::new(0));
    rt.spawn(async move {
        for _ in 0..12 {
            task::yield_now().await;
        }
        tx.send(()).unwrap();
    });

    rt.block_on(async {
        BlockedFuture {
            rx,
            num_polls: num_polls.clone(),
        }
        .await;
    });

    let polls = num_polls.load(Acquire);
    assert!(polls <= at_most_polls);
}

#[test]
fn block_on_num_polls() {
    loom::model(|| {
        // we expect at most 4 number of polls because there are three points at
        // which we poll the future and an opportunity for a false-positive.. At
        // any of these points it can be ready:
        //
        // - when we fail to steal the parker and we block on a notification
        //   that it is available.
        //
        // - when we steal the parker and we schedule the future
        //
        // - when the future is woken up and we have ran the max number of tasks
        //   for the current tick or there are no more tasks to run.
        //
        // - a thread is notified that the parker is available but a third
        //   thread acquires it before the notified thread can.
        //
        let at_most = 4;

        let rt1 = Arc::new(Builder::new_current_thread().build().unwrap());
        let rt2 = rt1.clone();
        let rt3 = rt1.clone();

        let th1 = thread::spawn(move || assert_at_most_num_polls(rt1, at_most));
        let th2 = thread::spawn(move || assert_at_most_num_polls(rt2, at_most));
        let th3 = thread::spawn(move || assert_at_most_num_polls(rt3, at_most));

        th1.join().unwrap();
        th2.join().unwrap();
        th3.join().unwrap();
    });
}

#[test]
fn assert_no_unnecessary_polls() {
    loom::model(|| {
        // // After we poll outer future, woken should reset to false
        let rt = Builder::new_current_thread().build().unwrap();
        let (tx, rx) = oneshot::channel();
        let pending_cnt = Arc::new(AtomicUsize::new(0));

        rt.spawn(async move {
            for _ in 0..24 {
                task::yield_now().await;
            }
            tx.send(()).unwrap();
        });

        let pending_cnt_clone = pending_cnt.clone();
        rt.block_on(async move {
            // use task::yield_now() to ensure woken set to true
            // ResetFuture will be polled at most once
            // Here comes two cases
            // 1. recv no message from channel, ResetFuture will be polled
            //    but get Pending and we record ResetFuture.pending_cnt ++.
            //    Then when message arrive, ResetFuture returns Ready. So we
            //    expect ResetFuture.pending_cnt = 1
            // 2. recv message from channel, ResetFuture returns Ready immediately.
            //    We expect ResetFuture.pending_cnt = 0
            task::yield_now().await;
            ResetFuture {
                rx,
                pending_cnt: pending_cnt_clone,
            }
            .await;
        });

        let pending_cnt = pending_cnt.load(Acquire);
        assert!(pending_cnt <= 1);
    });
}

#[repr(C)]
struct BlockedFuture {
    rx: Receiver<()>,
    num_polls: Arc<AtomicUsize>,
}

impl Future for BlockedFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.num_polls.fetch_add(1, Release);

        match Pin::new(&mut self.rx).poll(cx) {
            Poll::Pending => Poll::Pending,
            _ => Poll::Ready(()),
        }
    }
}

#[repr(C)]
struct ResetFuture {
    rx: Receiver<()>,
    pending_cnt: Arc<AtomicUsize>,
}

impl Future for ResetFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.rx).poll(cx) {
            Poll::Pending => {
                self.pending_cnt.fetch_add(1, Release);
                Poll::Pending
            }
            _ => Poll::Ready(()),
        }
    }
}
