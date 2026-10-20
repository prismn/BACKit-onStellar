use soroban_sdk::contracterror;

#[contracterror]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u32)]
pub enum ParlayError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidLegCount = 3,
    InvalidStakeAmount = 4,
    InvalidOutcome = 5,
    ParlayNotFound = 6,
    ParlayNotActive = 7,
    LegNotResolved = 8,
    MarketCallFailed = 9,
    ContractPaused = 10,
    /// Two legs reference the same market.
    DuplicateMarket = 11,
}
