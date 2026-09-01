//! Owns the rusqlite `Store` on a dedicated thread. `Connection` is not
//! Sync, so handlers send closures here instead of holding the store
//! across an await.

use scry_core::store::Store;

type Job = Box<dyn FnOnce(&mut Store) + Send>;

#[derive(Clone)]
pub struct StoreHandle {
    tx: std::sync::mpsc::Sender<Job>,
}

impl StoreHandle {
    pub fn spawn(store: Store) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        std::thread::spawn(move || {
            let mut store = store;
            for job in rx {
                job(&mut store);
            }
        });
        Self { tx }
    }

    pub async fn call<T, F>(&self, f: F) -> T
    where
        T: Send + 'static,
        F: FnOnce(&mut Store) -> T + Send + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Box::new(move |store| {
                let _ = tx.send(f(store));
            }))
            .expect("store thread alive");
        rx.await.expect("store thread alive")
    }
}
