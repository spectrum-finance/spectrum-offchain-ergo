# Spectrum Off-Chain Streams (Liquidity Mining)

```mermaid
flowchart TD
    CS{{Chain Upgrade Stream}}
        -->LS{{Ledger TX Event Stream}}
        -->CPS{{Confirmed Pool Stream}}

    subgraph chain_sync
        CS
    end

    CPS-->PR>Pool Tracker]
    LS-->PT>Program Tracker]
    LS-->ST>Schedule Tracker]

    LS
        -->OS{{Order Stream}}
        -->BL>Backlog]

    LS
        -->BS{{Bundle Stream}}
        -->BT>Bundle Tracker]

    LS
        -->FS{{Funding Stream}}
        -->FT>Funding Tracker]

    subgraph offchain_lm
        LS
        CPS
        PR
        PT
        ST
        OS
        BL
        BS
        BT
        FS
        FT
    end

```
