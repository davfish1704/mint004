# Deployment Instructions

Follow these steps to deploy the referral program to a Solana cluster.

1. **Generate build artifacts**
   - Install the Anchor CLI that matches `anchor-lang` 0.29.0.
   - Run `anchor build` from the repository root to produce `target/deploy/trsales_license.so` and the refreshed IDL.

2. **Prepare cluster configuration**
   - Select the target cluster (`devnet`, `testnet`, or `mainnet-beta`).
   - Ensure the deploying wallet has sufficient SOL to cover rent for the `Config` account, three early referral registrations, and treasury prefunding.

3. **Create supporting accounts**
   - Derive the PDA for the config account: `Pubkey.findProgramAddress(["config"], programId)`.
   - Create or select the NFT collection mint and set its mint & freeze authority to the config PDA.
   - Identify the treasury SOL address (may be an off-chain multisig) and the SPL token account that will custody USDC payouts.

4. **Deploy the program**
   - Use `solana program deploy target/deploy/trsales_license.so` and record the resulting program id.
   - Update `Anchor.toml`, the IDL metadata, and frontend environment variables with the deployed id.

5. **Initialize configuration**
   - Create the collection mint (if not already) and treasury USDC ATA before initialization.
   - Run the `initialize` instruction with `sol_price`, `usdc_price`, and the admin authority (recommended: multisig).
   - Verify the config account contents with `anchor account Config <config-pda>`.

6. **Smoke tests**
   - Register three test wallets and create a referral chain.
   - Execute `mintWithSol` and `mintWithUsdc` transactions using small amounts to confirm reward routing and receipt creation.
   - Test `claimSol` and `claimUsdc` to ensure PDAs sign correctly.

7. **Frontend and backend rollout**
   - Publish the new IDL (`target/idl/trsales_license.json`) to the frontend bundle or backend service.
   - Update API services to supply `order_id` seeds, referral remaining accounts, and token account addresses when invoking the program.
   - Monitor logs for the first production mints to confirm order receipts are unique and referral vault balances update as expected.

8. **Operational readiness**
   - Document emergency procedures for pausing sales (`updateConfig.paused = true`).
   - Establish alerting on treasury balances and unexpected errors emitted by the program.
   - Schedule regular reconciliation of on-chain referral earnings with any off-chain analytics dashboards.

