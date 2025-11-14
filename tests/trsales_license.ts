import * as anchor from "@coral-xyz/anchor";
import { BN, Program } from "@coral-xyz/anchor";
import { Keypair, PublicKey, SystemProgram } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountInstruction,
  createMint,
  getAccount,
  getAssociatedTokenAddressSync,
  mintTo,
} from "@solana/spl-token";
import { TrsalesLicense } from "../target/types/trsales_license";

const CONFIG_SEED = Buffer.from("config");
const PROFILE_SEED = Buffer.from("profile");
const REWARD_SEED = Buffer.from("reward");
const ORDER_SEED = Buffer.from("order");

describe("trsales_license", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.TrsalesLicense as Program<TrsalesLicense>;

  const connection = provider.connection;
  const admin = provider.wallet as anchor.Wallet;

  const solPrice = new BN(1_000_000_000); // 1 SOL
  const usdcPrice = new BN(1_000_000); // 1 USDC assuming 6 decimals

  let configPda: PublicKey;
  let usdcMint: PublicKey;
  let collectionMint: PublicKey;
  let treasuryUsdcAta: PublicKey;

  const top = Keypair.generate();
  const mid = Keypair.generate();
  const direct = Keypair.generate();
  const solBuyer = Keypair.generate();
  const usdcBuyer = Keypair.generate();

  const users = [top, mid, direct, solBuyer, usdcBuyer];

  before("setup program", async () => {
    configPda = PublicKey.findProgramAddressSync([CONFIG_SEED], program.programId)[0];

    usdcMint = await createMint(
      connection,
      admin.payer,
      admin.publicKey,
      null,
      6,
    );

    const treasuryUsdc = getAssociatedTokenAddressSync(usdcMint, admin.publicKey);
    const treasuryInfo = await connection.getAccountInfo(treasuryUsdc);
    if (!treasuryInfo) {
      const ix = createAssociatedTokenAccountInstruction(
        admin.publicKey,
        treasuryUsdc,
        admin.publicKey,
        usdcMint,
        TOKEN_PROGRAM_ID,
        ASSOCIATED_TOKEN_PROGRAM_ID,
      );
      const tx = new anchor.web3.Transaction().add(ix);
      await provider.sendAndConfirm(tx, []);
    }
    treasuryUsdcAta = treasuryUsdc;

    collectionMint = await createMint(
      connection,
      admin.payer,
      configPda,
      configPda,
      0,
    );

    await program.methods
      .initialize({
        admin: admin.publicKey,
        usdcMint,
        collectionMint,
        solPrice,
        usdcPrice,
      })
      .accounts({
        config: configPda,
        payer: admin.publicKey,
        treasurySol: admin.publicKey,
        treasuryUsdc: treasuryUsdcAta,
        collectionMint,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    for (const user of users) {
      const signature = await connection.requestAirdrop(user.publicKey, 3 * anchor.web3.LAMPORTS_PER_SOL);
      await connection.confirmTransaction(signature, "confirmed");
    }
  });

  const getProfilePda = (pk: PublicKey) =>
    PublicKey.findProgramAddressSync([PROFILE_SEED, pk.toBuffer()], program.programId)[0];

  const getRewardVaultPda = (pk: PublicKey) =>
    PublicKey.findProgramAddressSync([REWARD_SEED, pk.toBuffer()], program.programId)[0];

  const getOrderPda = (id: Buffer) =>
    PublicKey.findProgramAddressSync([ORDER_SEED, id], program.programId)[0];

  async function register(user: Keypair) {
    const profilePda = getProfilePda(user.publicKey);
    const rewardVault = getRewardVaultPda(user.publicKey);
    const rewardUsdc = getAssociatedTokenAddressSync(usdcMint, rewardVault, true);

    await program.methods
      .registerUser()
      .accounts({
        config: configPda,
        payer: user.publicKey,
        profile: profilePda,
        rewardVault,
        rewardUsdc,
        user: user.publicKey,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([user])
      .rpc();

    return { profilePda, rewardVault, rewardUsdc };
  }

  async function setReferrer(child: Keypair, parent: PublicKey) {
    const profilePda = getProfilePda(child.publicKey);
    const parentPda = getProfilePda(parent);
    await program.methods
      .setReferrer(parent)
      .accounts({
        profile: profilePda,
        user: child.publicKey,
        parentProfile: parentPda,
      })
      .signers([child])
      .rpc();
  }

  async function createCollectionAta(owner: PublicKey) {
    const ata = getAssociatedTokenAddressSync(collectionMint, owner);
    const info = await connection.getAccountInfo(ata);
    if (!info) {
      const ix = createAssociatedTokenAccountInstruction(
        admin.publicKey,
        ata,
        owner,
        collectionMint,
        TOKEN_PROGRAM_ID,
        ASSOCIATED_TOKEN_PROGRAM_ID,
      );
      const tx = new anchor.web3.Transaction().add(ix);
      await provider.sendAndConfirm(tx, []);
    }
    return ata;
  }

  const orderSeedFor = (label: string) => {
    const buf = Buffer.alloc(16);
    buf.write(label);
    return buf;
  };

  async function mintSolSale(buyer: Keypair, orderSeed: Buffer, remaining: anchor.web3.AccountMeta[]) {
    const profilePda = getProfilePda(buyer.publicKey);
    const rewardVault = getRewardVaultPda(buyer.publicKey);
    const buyerAta = getAssociatedTokenAddressSync(collectionMint, buyer.publicKey);
    const orderPda = getOrderPda(orderSeed);

    await program.methods
      .mintWithSol([...orderSeed])
      .accounts({
        config: configPda,
        treasurySol: admin.publicKey,
        buyer: buyer.publicKey,
        order: orderPda,
        buyerProfile: profilePda,
        buyerRewardVault: rewardVault,
        collectionMint,
        buyerNftAccount: buyerAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(remaining)
      .signers([buyer])
      .rpc();
  }

  async function mintUsdcSale(buyer: Keypair, orderSeed: Buffer, remaining: anchor.web3.AccountMeta[]) {
    const profilePda = getProfilePda(buyer.publicKey);
    const rewardVault = getRewardVaultPda(buyer.publicKey);
    const buyerAta = getAssociatedTokenAddressSync(collectionMint, buyer.publicKey);
    const buyerUsdc = getAssociatedTokenAddressSync(usdcMint, buyer.publicKey);
    const orderPda = getOrderPda(orderSeed);

    await program.methods
      .mintWithUsdc([...orderSeed])
      .accounts({
        config: configPda,
        treasurySol: admin.publicKey,
        treasuryUsdc: treasuryUsdcAta,
        buyer: buyer.publicKey,
        buyerUsdc,
        order: orderPda,
        buyerProfile: profilePda,
        buyerRewardVault: rewardVault,
        collectionMint,
        buyerNftAccount: buyerAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(remaining)
      .signers([buyer])
      .rpc();
  }

  async function referralRemainingAccounts(start: PublicKey, depth: number) {
    const accounts: anchor.web3.AccountMeta[] = [];
    let current: PublicKey | null = start;
    for (let i = 0; i < depth; i++) {
      if (!current) break;
      const profile = getProfilePda(current);
      const rewardVault = getRewardVaultPda(current);
      const rewardUsdc = getAssociatedTokenAddressSync(usdcMint, rewardVault, true);
      const nftAta = getAssociatedTokenAddressSync(collectionMint, current);
      accounts.push({ pubkey: profile, isSigner: false, isWritable: true });
      accounts.push({ pubkey: rewardVault, isSigner: false, isWritable: true });
      accounts.push({ pubkey: rewardUsdc, isSigner: false, isWritable: true });
      accounts.push({ pubkey: nftAta, isSigner: false, isWritable: false });
      const profileAccount = await program.account.referralProfile.fetch(profile);
      current = profileAccount.hasReferrer ? new PublicKey(profileAccount.referrer) : null;
    }
    return accounts;
  }

  it("registers referral tree and mints NFTs", async () => {
    await register(top);
    await register(mid);
    await register(direct);
    await register(solBuyer);
    await register(usdcBuyer);

    await setReferrer(mid, top.publicKey);
    await setReferrer(direct, mid.publicKey);
    await setReferrer(solBuyer, direct.publicKey);
    await setReferrer(usdcBuyer, direct.publicKey);

    for (const user of [top, mid, direct, solBuyer, usdcBuyer]) {
      await createCollectionAta(user.publicKey);
    }

    await mintSolSale(top, orderSeedFor("order-top-000001"), []);

    const midRemaining = await referralRemainingAccounts(top.publicKey, 1);
    await mintSolSale(mid, orderSeedFor("order-mid-000002"), midRemaining);

    const directRemaining = await referralRemainingAccounts(mid.publicKey, 2);
    await mintSolSale(direct, orderSeedFor("order-dir-000003"), directRemaining);
  });

  it("distributes SOL referral rewards to three levels", async () => {
    const remaining = await referralRemainingAccounts(direct.publicKey, 3);
    const orderSeed = orderSeedFor("order-sol-buy-04");
    await mintSolSale(solBuyer, orderSeed, remaining);

    const topVault = await program.account.rewardVault.fetch(getRewardVaultPda(top.publicKey));
    const midVault = await program.account.rewardVault.fetch(getRewardVaultPda(mid.publicKey));
    const directVault = await program.account.rewardVault.fetch(getRewardVaultPda(direct.publicKey));

    const l1 = solPrice.mul(new BN(50)).div(new BN(100)).toNumber();
    const l2 = solPrice.mul(new BN(30)).div(new BN(100)).toNumber();
    const l3 = solPrice.mul(new BN(20)).div(new BN(100)).toNumber();

    anchor.assert.equal(directVault.claimableSol.toNumber(), l1);
    anchor.assert.equal(midVault.claimableSol.toNumber(), l2);
    anchor.assert.equal(topVault.claimableSol.toNumber(), l3);
  });

  it("distributes USDC referral rewards and supports claims", async () => {
    const buyerUsdcAta = getAssociatedTokenAddressSync(usdcMint, usdcBuyer.publicKey);
    const info = await connection.getAccountInfo(buyerUsdcAta);
    if (!info) {
      const ix = createAssociatedTokenAccountInstruction(
        admin.publicKey,
        buyerUsdcAta,
        usdcBuyer.publicKey,
        usdcMint,
        TOKEN_PROGRAM_ID,
        ASSOCIATED_TOKEN_PROGRAM_ID,
      );
      const tx = new anchor.web3.Transaction().add(ix);
      await provider.sendAndConfirm(tx, []);
    }
    await mintTo(connection, admin.payer, usdcMint, buyerUsdcAta, admin.publicKey, usdcPrice.toNumber() * 5);

    const remaining = await referralRemainingAccounts(direct.publicKey, 3);
    const orderSeed = orderSeedFor("order-usdc-005");
    await mintUsdcSale(usdcBuyer, orderSeed, remaining);

    const topVault = await program.account.rewardVault.fetch(getRewardVaultPda(top.publicKey));
    const midVault = await program.account.rewardVault.fetch(getRewardVaultPda(mid.publicKey));
    const directVault = await program.account.rewardVault.fetch(getRewardVaultPda(direct.publicKey));

    const l1 = usdcPrice.mul(new BN(50)).div(new BN(100));
    const l2 = usdcPrice.mul(new BN(30)).div(new BN(100));
    const l3 = usdcPrice.mul(new BN(20)).div(new BN(100));

    anchor.assert.equal(directVault.claimableUsdc.toString(), l1.toString());
    anchor.assert.equal(midVault.claimableUsdc.toString(), l2.toString());
    anchor.assert.equal(topVault.claimableUsdc.toString(), l3.toString());

    const before = await connection.getBalance(direct.publicKey);
    await program.methods
      .claimSol()
      .accounts({
        rewardVault: getRewardVaultPda(direct.publicKey),
        user: direct.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([direct])
      .rpc();
    const after = await connection.getBalance(direct.publicKey);
    anchor.assert.ok(after > before);

    const directUsdcAta = getAssociatedTokenAddressSync(usdcMint, direct.publicKey);
    const directUsdcInfo = await connection.getAccountInfo(directUsdcAta);
    if (!directUsdcInfo) {
      const ix = createAssociatedTokenAccountInstruction(
        admin.publicKey,
        directUsdcAta,
        direct.publicKey,
        usdcMint,
        TOKEN_PROGRAM_ID,
        ASSOCIATED_TOKEN_PROGRAM_ID,
      );
      const tx = new anchor.web3.Transaction().add(ix);
      await provider.sendAndConfirm(tx, []);
    }
    const beforeUsdc = BigInt((await getAccount(connection, directUsdcAta)).amount.toString());
    await program.methods
      .claimUsdc()
      .accounts({
        rewardVault: getRewardVaultPda(direct.publicKey),
        rewardUsdc: getAssociatedTokenAddressSync(usdcMint, getRewardVaultPda(direct.publicKey), true),
        userUsdc: directUsdcAta,
        user: direct.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([direct])
      .rpc();
    const afterUsdc = BigInt((await getAccount(connection, directUsdcAta)).amount.toString());
    anchor.assert.ok(afterUsdc > beforeUsdc);
  });
});
