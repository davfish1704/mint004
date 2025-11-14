# Security Audit Checklist

Use this list to review the referral program before deploying to mainnet.

- [ ] **Program ID**: Confirm the deployed program id matches `Trsa1esReferr4l1ju8GhhQXUfdViQspuWqX9u9KQk8k` in `Anchor.toml`, the IDL, and frontend configuration.
- [ ] **Configuration authority**: Verify the `admin` key in the `Config` account is the intended multisig and that `treasury_sol` / `treasury_usdc` destinations are correct.
- [ ] **Collection mint**: Ensure the NFT collection mint uses the `config` PDA as mint and freeze authority prior to initialization.
- [ ] **USDC mint**: Double check the USDC mint public key for the target cluster (e.g. mainnet `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v`).
- [ ] **Referral registration**: Enforce that referral PDAs are created by end users (covers rent) and parents exist before linking.
- [ ] **Treasury balances**: After test mints, confirm undistributed rewards accrue to the treasury accounts.
- [ ] **Reward vault balances**: Inspect a sample of `RewardVault` accounts to ensure lamport balances match `claimable_sol` and token balances match `claimable_usdc`.
- [ ] **CPI guards**: Confirm that only expected programs are passed in each instruction (`system_program`, `token_program`, `associated_token_program`).
- [ ] **Order receipts**: Validate that unique `order_id` values are supplied from the backend to prevent double settlement.
- [ ] **Withdrawal flows**: Exercise `claim_sol` and `claim_usdc` on devnet to confirm signer checks and PDA seeds are correct.
- [ ] **Access controls**: Attempt to call admin-only instructions (`update_config`) from a non-admin wallet and ensure they fail.
- [ ] **Frontend integration**: Update the frontend to use the on-chain IDL and confirm referral discovery populates remaining accounts in the correct order.
- [ ] **Monitoring**: Set up transaction log ingestion for the `OrderReceipt` account events to monitor settlement anomalies.

