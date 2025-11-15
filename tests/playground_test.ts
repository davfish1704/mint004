import { workspace, Program } from "@coral-xyz/anchor";

(async () => {
  console.log("🚀 Starting TRSales License live test...");

  const program = workspace.trsales_license as Program;

  console.log("Program ID:", program.programId.toBase58());

  // Call initialize instruction:
  try {
    const tx = await program.methods
      .initializeProgram()
      .rpc();

    console.log("✅ initializeProgram successful");
    console.log("TX:", tx);
  } catch (err) {
    console.error("❌ Error calling initializeProgram:", err);
  }

})();
