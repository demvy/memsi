use super::*;
use std::time::Duration;

fn request() -> CandleRequest {
    let chain = ChainId::Evm { chain_id: 1 };
    CandleRequest {
        pool: PoolId::address(chain.clone(), "0x1111111111111111111111111111111111111111").unwrap(),
        priced_token: TokenId::new(chain, "0x2222222222222222222222222222222222222222").unwrap(),
        price_unit: PriceUnit::UsdPerToken,
        resolution: Resolution::new(60).unwrap(),
        from_inclusive: TimestampMs::new(0).unwrap(),
        end_exclusive: TimestampMs::new(120_000).unwrap(),
        min_bars: 1,
        max_bars: 2,
    }
}

#[test]
fn cancellation_is_shared_and_priority_promotion_can_be_independent() {
    let cancellation = Cancellation::default();
    let shared = cancellation.clone();
    assert!(!shared.is_cancelled());
    cancellation.cancel();
    assert!(shared.is_cancelled());
    shared.cancel();
    assert!(cancellation.is_cancelled());

    assert!(RequestPriority::default().is_active());
    let priority = RequestPriority::shadow();
    let shared = priority.clone();
    let independent = priority.independent();
    assert!(!priority.is_active());
    independent.promote_active();
    assert!(independent.is_active());
    assert!(!priority.is_active());
    priority.promote_active();
    assert!(shared.is_active());
    assert!(priority.independent().is_active());
}

#[test]
fn request_context_prioritizes_cancellation_over_expired_deadlines() {
    let mut context = RequestContext {
        priority: RequestPriority::default(),
        deadline: Instant::now() + Duration::from_secs(60),
        cancellation: Cancellation::default(),
    };
    assert_eq!(context.check(), Ok(()));
    context.deadline = Instant::now();
    assert_eq!(context.check(), Err(ProviderError::Timeout));
    context.cancellation.cancel();
    assert_eq!(context.check(), Err(ProviderError::Cancelled));
}

#[test]
fn candle_requests_check_bar_bounds_alignment_time_and_chain() {
    let valid = request();
    assert!(valid.validate().is_ok());
    for field in 0..8 {
        let mut invalid = valid.clone();
        match field {
            0 => invalid.min_bars = 0,
            1 => invalid.min_bars = 3,
            2 => invalid.max_bars = 10001,
            3 => invalid.from_inclusive = invalid.end_exclusive,
            4 => invalid.from_inclusive = TimestampMs::new(180_000).unwrap(),
            5 => invalid.from_inclusive = TimestampMs::new(1).unwrap(),
            6 => invalid.end_exclusive = TimestampMs::new(120_001).unwrap(),
            _ => {
                invalid.priced_token =
                    TokenId::new(ChainId::Evm { chain_id: 2 }, valid.priced_token.address())
                        .unwrap()
            }
        }

        assert_eq!(invalid.validate(), Err(ProviderError::InvalidRequest));
    }
}

#[test]
fn domain_errors_retain_their_message_when_converted() {
    let error = DomainError::Market("invalid candle");
    assert_eq!(
        ProviderError::from(error),
        ProviderError::InvalidData("invalid market data: invalid candle".into())
    );
}

struct DefaultProvider;
#[async_trait]
impl PoolDiscoveryProvider for DefaultProvider {
    async fn pools_for_token(
        &self,
        _: &TokenId,
        _: &RequestContext,
    ) -> Result<PoolDiscoveryResult, ProviderError> {
        Err(ProviderError::NotFound)
    }
}

#[async_trait]
impl TransportObservationReader for DefaultProvider {
    fn transport_observations(&self) -> Option<TransportObservations> {
        None
    }
}

#[tokio::test]
async fn default_metric_methods_return_empty_facts_and_validate_batch_identity_and_context() {
    let provider = DefaultProvider;
    let context = RequestContext {
        priority: RequestPriority::default(),
        deadline: Instant::now() + Duration::from_secs(60),
        cancellation: Cancellation::default(),
    };
    let token = request().priced_token;
    assert!(
        provider
            .token_metrics(&token, &context)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        provider
            .token_metrics_selected(&token, &[TokenMetric::MarketCapUsd], &context)
            .await
            .unwrap()
            .is_empty()
    );
    for fields in [None, Some(&[TokenMetric::MarketCapUsd][..])] {
        let result = provider
            .token_metrics_batch(std::slice::from_ref(&token), fields, &context)
            .await
            .unwrap();
        assert!(result[&token].is_empty());
    }

    for tokens in [vec![], vec![token.clone(); 2], vec![token.clone(); 101]] {
        assert_eq!(
            provider
                .token_metrics_batch(&tokens, None, &context)
                .await
                .unwrap_err(),
            ProviderError::InvalidRequest
        );
    }

    context.cancellation.cancel();
    assert_eq!(
        provider
            .token_metrics_batch(&[token], None, &context)
            .await
            .unwrap_err(),
        ProviderError::Cancelled
    );
    assert!(provider.api_usage().await.unwrap().is_none());
    let usage = ApiKeyUsage {
        key_id: "id".into(),
        period: "2025-01-01".into(),
        used: 1,
        limit: 10,
        warn_at: 9,
    };
    assert!(!provider.claim_usage_notice(&usage, false).await.unwrap());
}

#[test]
fn cancellation_wakes_registered_waiters_and_remains_ready_for_late_waiters() {
    use std::{
        future::Future,
        sync::atomic::AtomicUsize,
        task::{Context, Wake, Waker},
    };

    #[derive(Default)]
    struct WakeCounter(AtomicUsize);

    impl Wake for WakeCounter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    let cancellation = Cancellation::default();
    let shared = cancellation.clone();
    let counter = Arc::new(WakeCounter::default());
    let waker = Waker::from(counter.clone());
    let mut context = Context::from_waker(&waker);
    let mut first = std::pin::pin!(cancellation.cancelled());
    let mut second = std::pin::pin!(shared.cancelled());
    assert!(first.as_mut().poll(&mut context).is_pending());
    assert!(second.as_mut().poll(&mut context).is_pending());

    shared.cancel();

    assert_eq!(counter.0.load(Ordering::Relaxed), 2);
    assert!(first.as_mut().poll(&mut context).is_ready());
    assert!(second.as_mut().poll(&mut context).is_ready());
    assert!(
        std::pin::pin!(shared.cancelled())
            .as_mut()
            .poll(&mut context)
            .is_ready()
    );
}
