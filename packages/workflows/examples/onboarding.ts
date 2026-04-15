import { workflow, createRunner, type WorkflowContext } from "../src/index";

// ---------------------------------------------------------------------------
// Workflow: User onboarding
// ---------------------------------------------------------------------------

const onboardingFlow = workflow(
  "user-onboarding",
  async (ctx, { step, sleep, waitForEvent }) => {
    // Step 1: Send welcome email
    const welcomeResult = await step("send-welcome-email", async () => {
      console.log(`Sending welcome email to ${ctx.input.email}`);
      // In real app: await emailService.send(ctx.input.email, 'Welcome!');
      return { sent: true, email: ctx.input.email };
    });

    // Step 2: Wait 24 hours
    await sleep("24h");

    // Step 3: Check profile completion
    const profileComplete = await step("check-profile", async () => {
      console.log(`Checking profile for user ${ctx.input.userId}`);
      // In real app: const user = await db.query('User', { id: ctx.input.userId });
      return { completed: Math.random() > 0.5 }; // Simulated
    });

    // Step 4: Conditional action
    if (!profileComplete.completed) {
      await step("send-reminder", async () => {
        console.log("Sending reminder email");
        return { reminded: true };
      });

      // Wait for user to complete profile
      const event = await waitForEvent("profile_completed");
      console.log("Profile completed event received:", event);
    }

    // Step 5: Final step
    await step("activate-features", async () => {
      console.log("Activating premium features");
      return { activated: true };
    });

    return { onboarding: "complete", user: ctx.input.userId };
  },
);

// ---------------------------------------------------------------------------
// Workflow: Data processing pipeline
// ---------------------------------------------------------------------------

const dataProcessingFlow = workflow(
  "data-processing",
  async (ctx, { step, sleep }) => {
    const data = await step("fetch-data", async () => {
      console.log(`Fetching data from ${ctx.input.source}`);
      return { rows: 1000, source: ctx.input.source };
    });

    await step("validate", async () => {
      console.log(`Validating ${data.rows} rows`);
      return { valid: data.rows, invalid: 0 };
    });

    await step("transform", async () => {
      console.log("Transforming data");
      return { transformed: data.rows };
    });

    // Pause between steps to avoid rate limits
    await sleep("5s");

    await step("load", async () => {
      console.log("Loading into database");
      return { loaded: data.rows };
    });

    return { processed: data.rows, source: ctx.input.source };
  },
);

// ---------------------------------------------------------------------------
// Start the runner
// ---------------------------------------------------------------------------

const runner = createRunner([onboardingFlow, dataProcessingFlow]);
runner.serve(4500);
