use super::*;

impl Monitor<'_> {
    /// Bounded sequential cycles with the normal one-second scheduler idle.
    pub async fn run_cycles(
        &mut self,
        cancellation: &Cancellation,
        max_cycles: u16,
        mut on_cycle: impl FnMut(&CycleReport),
    ) -> Result<usize, StateError> {
        if !(1..=100).contains(&max_cycles) {
            return Err(StateError::InvalidRequest);
        }
        let result = AssertUnwindSafe(async {
            let mut completed = 0;
            while completed < usize::from(max_cycles) && !cancellation.is_cancelled() {
                let report = self.cycle(cancellation).await?;
                on_cycle(&report);
                completed += 1;
                if completed < usize::from(max_cycles) {
                    idle(cancellation, 1000).await;
                }
            }
            Ok(completed)
        })
        .catch_unwind()
        .await
        .unwrap_or_else(|_| {
            cancellation.cancel();
            Err(StateError::Unavailable)
        });
        self.status.finish_run(result.is_err());
        result
    }

    /// Continuous analysis and delivery share state, but never wait for each other's I/O.
    /// The host must bound shutdown if a provider or repository stops responding.
    pub async fn run(
        &mut self,
        cancellation: &Cancellation,
        mut on_cycle: impl FnMut(&CycleReport),
    ) -> Result<(), StateError> {
        let state = self.dependencies.state;
        let sinks = self.dependencies.sinks;
        let clock = self.dependencies.clock;
        let signal_mode = self.dependencies.config.mode == AppMode::Signal;
        let events = self.events.clone();
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
            .unwrap_or(Err(StateError::Unavailable));
            cancellation.cancel();
            result
        };
        let delivery = async {
            let result = AssertUnwindSafe(async {
                while !cancellation.is_cancelled() {
                    if !signal_mode
                        || !signal_service::delivery::deliver_one_with_events(
                            state,
                            sinks,
                            clock,
                            cancellation,
                            &DeliveryPolicy::default(),
                            events.as_ref(),
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
            .unwrap_or(Err(StateError::Unavailable));
            cancellation.cancel();
            result
        };
        // Unlike try_join, preserve the other future until its in-flight operation settles.
        let (analysis, delivery) = futures::join!(analysis, delivery);
        let _ = self.events.close();
        let result = analysis.and(delivery);
        self.status.finish_run(result.is_err());
        result
    }
}

async fn idle(cancellation: &Cancellation, millis: u64) {
    let until = Instant::now() + Duration::from_millis(millis);
    while !cancellation.is_cancelled() && Instant::now() < until {
        tokio::time::sleep(
            until
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(25)),
        )
        .await;
    }
}
