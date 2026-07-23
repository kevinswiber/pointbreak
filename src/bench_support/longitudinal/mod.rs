//! Public, deterministic longitudinal benchmark contracts.
//!
//! Materialization and evidence execution are added by later source tasks. This
//! task freezes only the typed, non-timing contract consumed by those tasks.

mod contract;

pub use contract::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longitudinal_contract_freezes_disjoint_workload_identities() {
        let workload = longitudinal_runner_contract_v1();
        let capacity = longitudinal_capacity_contract_v1();

        assert_eq!(workload.schema, "pointbreak.longitudinal-workload.v1");
        assert_eq!(workload.protocol, "pointbreak.longitudinal-q3.v1");
        assert_eq!(
            workload.public_seed_hex,
            "f4da49601a212010bae444e6ca2de6c6bf28b5ec1b0a05bf42154a533ca513ff"
        );
        assert_eq!(
            workload
                .tiers
                .iter()
                .map(|tier| (tier.tier, tier.event_count, tier.revision_count))
                .collect::<Vec<_>>(),
            vec![
                (LongitudinalTierV1::L1, 1_024, 48),
                (LongitudinalTierV1::L7, 7_168, 336),
                (LongitudinalTierV1::L25, 25_600, 1_200),
                (LongitudinalTierV1::L100, 102_400, 4_800),
            ]
        );
        assert_eq!(
            capacity.schema,
            "pointbreak.longitudinal-capacity-sentinel.v1"
        );
        assert_ne!(workload.schema, capacity.schema);
    }

    #[test]
    fn longitudinal_contract_freezes_capacity_profiles_and_probes() {
        let contract = longitudinal_capacity_contract_v1();

        assert_eq!(
            contract
                .profiles
                .iter()
                .map(|profile| (profile.profile, profile.event_count, profile.revision_count))
                .collect::<Vec<_>>(),
            vec![
                (LongitudinalCapacityProfileV1::L100O10K, 102_400, 10_000),
                (LongitudinalCapacityProfileV1::C262, 262_144, 12_288),
                (LongitudinalCapacityProfileV1::C524, 524_288, 24_576),
            ]
        );
        assert_eq!(
            contract.probes,
            vec![
                LongitudinalCapacityProbeV1::CarrierKey,
                LongitudinalCapacityProbeV1::SemanticId,
                LongitudinalCapacityProbeV1::ChronologicalHead,
                LongitudinalCapacityProbeV1::ChronologicalMiddle,
                LongitudinalCapacityProbeV1::ChronologicalTail,
                LongitudinalCapacityProbeV1::ObjectDetail,
                LongitudinalCapacityProbeV1::AppendDelta,
            ]
        );
        assert_eq!(contract.non_compensation.maximum_ratio_basis_points, 12_500);
        assert_eq!(contract.contention.writer_processes, 2);
        assert_eq!(contract.contention.reader_processes, 1);
        assert_eq!(contract.contention.expected_created, 10);
        assert_eq!(contract.contention.expected_existing, 2);
        assert_eq!(contract.contention.reader_cycles, 20);
    }
}
