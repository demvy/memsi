use super::*;

impl Monitor<'_> {
    /// Continuous analysis and delivery share state, but never wait for each other's I/O.
    /// The host must bound shutdown if a provider or repository stops responding.
    pub async fn run(
        &mut self,
        cancellation: &Cancellation,
        mut on_cycle: impl FnMut(&CycleReport),
    ) -> Result<(), MonitorError> {
        let state = self.dependencies.state;
        let sinks = self.dependencies.sinks;
        let clock = self.dependencies.clock;
        let signal_mode = self.dependencies.config.mode == AppMode::Signal;
        let analysis = async {
            let result = AssertUnwindSafe(async {
                while !cancellation.is_cancelled() {
                    let report = self.cycle_with_delivery(cancellation, false).await?;
                    on_cycle(&report);
                    idle(cancellation, 1000).await;
                }

                Ok(())
            })
            .catch_unwind()
            .await
            .unwrap_or(Err(MonitorError::State(StateError::Unavailable)));
            cancellation.cancel();
            result
        };

        let delivery = async {
            let result = AssertUnwindSafe(async {
                while !cancellation.is_cancelled() {
                    if !signal_mode
                        || !signal_service::delivery::deliver_one(
                            state,
                            sinks,
                            clock,
                            cancellation,
                            &DeliveryPolicy::default(),
                        )
                        .await?
                    {
                        idle(cancellation, 100).await;
                    } else {
                        tokio::task::yield_now().await;
                    }
                }

                Ok(())
            })
            .catch_unwind()
            .await
            .unwrap_or(Err(MonitorError::State(StateError::Unavailable)));
            cancellation.cancel();
            result
        };
        // Unlike try_join, preserve the other future until its in-flight operation settles.
        let (analysis, delivery) = futures::join!(analysis, delivery);
        analysis.and(delivery)
    }
}

async fn idle(cancellation: &Cancellation, millis: u64) {
    tokio::select! {
        _ = cancellation.cancelled() => (),
        _ = tokio::time::sleep(Duration::from_millis(millis)) => (),
    }
}
