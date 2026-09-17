// Demo data for a brand-new helpdesk.
//
// The first sign-in seeds a working queue: twenty customers, forty tickets
// across every status and priority, and a message thread on each. Two open
// tickets are already past their first-response window, so the breach state is
// visible rather than theoretical.
//
// Pure data + pure shaping; `functions/seedWorkspace.ts` stays a thin wrapper.

export interface SeedCustomer {
  key: string;
  name: string;
  email: string;
  company: string;
}

export interface SeedTicket {
  key: string;
  customer: string;
  subject: string;
  status: string;
  priority: string;
  /** Hours since it arrived. */
  age: number;
  /** Hours since the first agent reply; null means nobody has answered. */
  respondedAgo: number | null;
}

export interface SeedMessage {
  ticket: string;
  body: string;
  fromCustomer: boolean;
  internal?: boolean;
  /** Hours ago. */
  age: number;
}

export const SEED_CUSTOMERS: SeedCustomer[] = [
  { key: "dana", name: "Dana Whitfield", email: "dana@northwind.co", company: "Northwind Logistics" },
  { key: "marcus", name: "Marcus Iyer", email: "marcus@northwind.co", company: "Northwind Logistics" },
  { key: "tom", name: "Tom Alvarez", email: "tom@riverbed.coffee", company: "Riverbed Coffee" },
  { key: "priya", name: "Priya Raman", email: "priya@hallmarkdental.com", company: "Hallmark Dental" },
  { key: "ben", name: "Ben Osei", email: "ben@quarry.design", company: "Quarry Design Co" },
  { key: "lena", name: "Lena Fischer", email: "lena@beaconproperty.com", company: "Beacon Property" },
  { key: "kevin", name: "Kevin Doyle", email: "kdoyle@larkspurclinics.com", company: "Larkspur Clinics" },
  { key: "rachel", name: "Rachel Stone", email: "rstone@meridianfreight.com", company: "Meridian Freight" },
  { key: "mike", name: "Mike Brennan", email: "mike@copperline.beer", company: "Copperline Brewing" },
  { key: "sarah", name: "Sarah Lindqvist", email: "sarah@tidewatermarine.com", company: "Tidewater Marine" },
  { key: "isabel", name: "Isabel Moreau", email: "isabel@sableandfinch.com", company: "Sable & Finch" },
  { key: "david", name: "David Park", email: "dpark@orchardlearning.org", company: "Orchard Learning" },
  { key: "hannah", name: "Hannah Weiss", email: "hannah@granitepeak.com", company: "Granite Peak Outfitters" },
  { key: "james", name: "James Whitaker", email: "jwhitaker@kestrelaviation.com", company: "Kestrel Aviation Services" },
  { key: "elena", name: "Elena Vasquez", email: "elena@bluefin.io", company: "Bluefin Analytics" },
  { key: "robert", name: "Robert Chen", email: "rchen@harborpointhotels.com", company: "Harbor Point Hotels" },
  { key: "patricia", name: "Patricia Nguyen", email: "pnguyen@summitcu.org", company: "Summit Credit Union" },
  { key: "walt", name: "Walt Jensen", email: "walt@pinecrestfarms.com", company: "Pinecrest Farms" },
  { key: "anita", name: "Anita Rao", email: "anita@vantagesolar.com", company: "Vantage Solar" },
  { key: "tara", name: "Tara Singh", email: "tara@cobaltstaffing.com", company: "Cobalt Staffing" },
];

export const SEED_TICKETS: SeedTicket[] = [
  // Open, unanswered. The first two are past their first-response target.
  { key: "export", customer: "dana", subject: "Export is timing out on large date ranges", status: "open", priority: "urgent", age: 3, respondedAgo: null },
  { key: "sso", customer: "ben", subject: "SSO with Google Workspace: is it supported?", status: "open", priority: "normal", age: 30, respondedAgo: null },
  { key: "login-loop", customer: "rachel", subject: "Login redirects back to the sign-in page", status: "open", priority: "high", age: 2, respondedAgo: null },
  { key: "csv-dates", customer: "walt", subject: "CSV import reads dates as US format", status: "open", priority: "normal", age: 5, respondedAgo: null },
  { key: "role-perms", customer: "patricia", subject: "Viewer role can still edit custom fields", status: "open", priority: "high", age: 1, respondedAgo: null },
  { key: "dark-mode", customer: "hannah", subject: "Any plans for a dark theme?", status: "open", priority: "low", age: 9, respondedAgo: null },
  { key: "api-limit", customer: "elena", subject: "Hitting 429s on the reports endpoint", status: "open", priority: "high", age: 4, respondedAgo: null },
  { key: "duplicate-contacts", customer: "tara", subject: "Duplicate contacts after ATS sync", status: "open", priority: "normal", age: 12, respondedAgo: null },

  // Open, answered, still with us.
  { key: "invite", customer: "tom", subject: "Can't invite a second manager to the account", status: "open", priority: "high", age: 6, respondedAgo: 5 },
  { key: "pdf-fonts", customer: "priya", subject: "PDF statements render with the wrong font", status: "open", priority: "normal", age: 20, respondedAgo: 17 },
  { key: "webhook-order", customer: "marcus", subject: "Webhook events arrive out of order", status: "open", priority: "high", age: 28, respondedAgo: 26 },
  { key: "mobile-upload", customer: "anita", subject: "Photo upload fails from the field app on iOS", status: "open", priority: "urgent", age: 7, respondedAgo: 6.5 },
  { key: "search-accents", customer: "isabel", subject: "Search does not match names with accents", status: "open", priority: "normal", age: 40, respondedAgo: 36 },
  { key: "timezone", customer: "robert", subject: "Scheduled reports send at the wrong hour", status: "open", priority: "normal", age: 50, respondedAgo: 45 },
  { key: "bulk-delete", customer: "david", subject: "Bulk delete stops after 200 rows", status: "open", priority: "normal", age: 60, respondedAgo: 58 },
  { key: "audit-log", customer: "kevin", subject: "Audit log missing permission changes", status: "open", priority: "high", age: 33, respondedAgo: 31 },

  // Pending on the customer.
  { key: "invoice", customer: "priya", subject: "Invoice shows last month's plan", status: "pending", priority: "normal", age: 26, respondedAgo: 24 },
  { key: "domain", customer: "mike", subject: "Custom domain stuck on pending", status: "pending", priority: "normal", age: 72, respondedAgo: 70 },
  { key: "seat-count", customer: "james", subject: "Seat count on the invoice is higher than our users", status: "pending", priority: "normal", age: 90, respondedAgo: 88 },
  { key: "ssl-warning", customer: "sarah", subject: "Browser shows a certificate warning on the portal", status: "pending", priority: "high", age: 48, respondedAgo: 47 },
  { key: "gdpr-export", customer: "lena", subject: "Need a full data export for a GDPR request", status: "pending", priority: "normal", age: 100, respondedAgo: 96 },
  { key: "calendar-sync", customer: "kevin", subject: "Outlook calendar sync creates duplicates", status: "pending", priority: "normal", age: 130, respondedAgo: 128 },
  { key: "training", customer: "patricia", subject: "Onboarding session for the new branch team", status: "pending", priority: "low", age: 150, respondedAgo: 140 },

  // Solved.
  { key: "mobile", customer: "lena", subject: "Mobile layout cuts off the listing photos", status: "solved", priority: "low", age: 120, respondedAgo: 110 },
  { key: "password-reset", customer: "hannah", subject: "Password reset email never arrives", status: "solved", priority: "high", age: 200, respondedAgo: 199 },
  { key: "report-totals", customer: "elena", subject: "Monthly report total does not match the dashboard", status: "solved", priority: "normal", age: 240, respondedAgo: 230 },
  { key: "slack", customer: "tara", subject: "Slack notifications stopped after the workspace rename", status: "solved", priority: "normal", age: 260, respondedAgo: 255 },
  { key: "print", customer: "walt", subject: "Printing a pick list cuts off the last column", status: "solved", priority: "low", age: 300, respondedAgo: 280 },
  { key: "two-factor", customer: "robert", subject: "Lost authenticator device, need 2FA reset", status: "solved", priority: "urgent", age: 320, respondedAgo: 319.5 },
  { key: "currency", customer: "sarah", subject: "Show amounts in SEK instead of USD", status: "solved", priority: "normal", age: 360, respondedAgo: 350 },
  { key: "attachment-size", customer: "david", subject: "Cannot attach files over 10 MB", status: "solved", priority: "normal", age: 400, respondedAgo: 390 },

  // Closed.
  { key: "webhook", customer: "dana", subject: "Webhook retries are firing twice", status: "closed", priority: "high", age: 500, respondedAgo: 490 },
  { key: "billing-address", customer: "mike", subject: "Update the billing address on our account", status: "closed", priority: "low", age: 520, respondedAgo: 515 },
  { key: "old-import", customer: "tom", subject: "Import from the old system dropped some notes", status: "closed", priority: "normal", age: 600, respondedAgo: 590 },
  { key: "api-key", customer: "marcus", subject: "Rotate the API key for the warehouse integration", status: "closed", priority: "high", age: 650, respondedAgo: 648 },
  { key: "welcome", customer: "isabel", subject: "Where do I change the welcome email text?", status: "closed", priority: "low", age: 700, respondedAgo: 690 },
  { key: "rename-workspace", customer: "james", subject: "Rename our workspace", status: "closed", priority: "low", age: 720, respondedAgo: 715 },
  { key: "downtime", customer: "rachel", subject: "Portal was unreachable for 20 minutes this morning", status: "closed", priority: "urgent", age: 800, respondedAgo: 799.5 },
  { key: "cancel-trial", customer: "anita", subject: "Cancel the second trial workspace we created by mistake", status: "closed", priority: "normal", age: 900, respondedAgo: 880 },
];

export const SEED_MESSAGES: SeedMessage[] = [
  { ticket: "export", body: "Pulling a 90-day export just spins and eventually errors. 30 days is fine. This is blocking our month-end reporting.", fromCustomer: true, age: 3 },
  { ticket: "sso", body: "We are standardising on Google Workspace this quarter. Does your plan include SAML, or only OAuth sign-in?", fromCustomer: true, age: 30 },
  { ticket: "login-loop", body: "Since this morning, signing in sends me straight back to the login page. Cleared cookies, same thing. Two colleagues report the same.", fromCustomer: true, age: 2 },
  { ticket: "csv-dates", body: "Uploaded our delivery schedule and every date after the 12th of the month got rejected. We write dates day first.", fromCustomer: true, age: 5 },
  { ticket: "role-perms", body: "A user with the Viewer role changed the value of a custom field on a member record. That should not be possible. Please treat as urgent, we have an audit next week.", fromCustomer: true, age: 1 },
  { ticket: "dark-mode", body: "Our warehouse team uses the app at night and the white screens are hard on the eyes. Is a dark theme on the roadmap?", fromCustomer: true, age: 9 },
  { ticket: "api-limit", body: "Our nightly job started getting 429 responses on /reports around 02:00 UTC. We make about 1,200 calls in that window. Did the limit change?", fromCustomer: true, age: 4 },
  { ticket: "duplicate-contacts", body: "After enabling the ATS sync we now have two records for most candidates. One has the email in lowercase and one in mixed case.", fromCustomer: true, age: 12 },

  { ticket: "invite", body: "I added marcus@northwind.co but he never got the email and does not show in the members list.", fromCustomer: true, age: 6 },
  { ticket: "invite", body: "Thanks Tom. I can see the invite was created but the delivery bounced. Re-sending now and checking the domain's SPF record.", fromCustomer: false, age: 5 },
  { ticket: "invite", body: "Their MX is misconfigured. Flagging for the infra team rather than sending again blind.", fromCustomer: false, internal: true, age: 5 },
  { ticket: "invite", body: "Still nothing on his end. Can you add him manually?", fromCustomer: true, age: 2 },

  { ticket: "pdf-fonts", body: "The statements we send patients now render in a serif font instead of our configured one. Started this week.", fromCustomer: true, age: 20 },
  { ticket: "pdf-fonts", body: "Confirmed on our side. The font file on the CDN was replaced with a broken build on Monday. A fix is rolling out. Can you send one affected PDF so I can verify?", fromCustomer: false, age: 17 },
  { ticket: "pdf-fonts", body: "Attached. Also noticed the page numbers are off by one.", fromCustomer: true, age: 10 },

  { ticket: "webhook-order", body: "We receive order.updated before order.created for about 5% of orders. Our consumer rejects the update because the record does not exist yet.", fromCustomer: true, age: 28 },
  { ticket: "webhook-order", body: "Ordering is not guaranteed across retries. I have written up how to buffer on the sequence number in the payload. Would that work for your consumer?", fromCustomer: false, age: 26 },
  { ticket: "webhook-order", body: "Engineering is looking at per-resource ordering. No ETA yet.", fromCustomer: false, internal: true, age: 26 },

  { ticket: "mobile-upload", body: "Crews cannot upload site photos from the iOS app since the update this morning. The spinner runs then the photo disappears. Android is fine.", fromCustomer: true, age: 7 },
  { ticket: "mobile-upload", body: "Reproduced on iOS 18.6 with HEIC photos. JPEG works. We are shipping a hotfix. In the meantime, set the camera to Most Compatible in iOS settings.", fromCustomer: false, age: 6.5 },
  { ticket: "mobile-upload", body: "That works as a workaround. Please let me know when the fix ships.", fromCustomer: true, age: 4 },

  { ticket: "search-accents", body: "Searching for Moreau finds me but searching for Béatrice returns nothing. Half our customers have accented names.", fromCustomer: true, age: 40 },
  { ticket: "search-accents", body: "Search currently matches exact characters. I have logged this as a bug. Typing the accented character finds the record for now.", fromCustomer: false, age: 36 },

  { ticket: "timezone", body: "Our Monday report is scheduled for 07:00 but arrives at 03:00. We are in Boston.", fromCustomer: true, age: 50 },
  { ticket: "timezone", body: "The schedule is stored in UTC and your workspace timezone was unset. I set it to America/New_York. Next Monday's report should arrive at 07:00 local.", fromCustomer: false, age: 45 },

  { ticket: "bulk-delete", body: "Selected 850 old enrolment records and chose Delete. It removed 200 and stopped with no error.", fromCustomer: true, age: 60 },
  { ticket: "bulk-delete", body: "Bulk actions are capped at 200 per request. The UI should say so. I have raised it. You can run the delete four more times or I can run it for you.", fromCustomer: false, age: 58 },
  { ticket: "bulk-delete", body: "Please run it for me. Everything with status Withdrawn before 2025.", fromCustomer: true, age: 40 },

  { ticket: "audit-log", body: "We changed a user's role from Admin to Member last week and nothing appears in the audit log for it.", fromCustomer: true, age: 33 },
  { ticket: "audit-log", body: "Role changes made through the members page are logged. Changes made through SCIM are not, which looks like your case. Confirming with engineering.", fromCustomer: false, age: 31 },

  { ticket: "invoice", body: "We upgraded on the 3rd but the invoice still lists the old plan. Can you re-issue?", fromCustomer: true, age: 26 },
  { ticket: "invoice", body: "Re-issued and emailed. The proration lands on next month's invoice.", fromCustomer: false, age: 24 },

  { ticket: "domain", body: "Added portal.copperline.beer three days ago and it still says Pending.", fromCustomer: true, age: 72 },
  { ticket: "domain", body: "The CNAME is pointing at the old target. It needs to be custom.stack0.app. Once that propagates the certificate issues itself.", fromCustomer: false, age: 70 },

  { ticket: "seat-count", body: "The invoice bills 42 seats but we only have 35 people in the members list.", fromCustomer: true, age: 90 },
  { ticket: "seat-count", body: "Pending invites count as seats until they are revoked. You have 7 outstanding invites from March. Revoke them and the next invoice will show 35.", fromCustomer: false, age: 88 },

  { ticket: "ssl-warning", body: "Staff see a certificate warning when opening the portal from the factory network.", fromCustomer: true, age: 48 },
  { ticket: "ssl-warning", body: "Our certificate is valid. This is usually a TLS-inspecting proxy on the local network replacing it. Can you send the certificate details the browser shows?", fromCustomer: false, age: 47 },

  { ticket: "gdpr-export", body: "A former tenant has requested all data we hold on them. We need an export of every record referencing their email.", fromCustomer: true, age: 100 },
  { ticket: "gdpr-export", body: "I can produce that. Please confirm the email address and whether you also want attachments included.", fromCustomer: false, age: 96 },

  { ticket: "calendar-sync", body: "Every appointment now appears twice in Outlook. Once from the sync and once from the email invite.", fromCustomer: true, age: 130 },
  { ticket: "calendar-sync", body: "Both the sync and the invite are enabled on your account. Turn off Send calendar invites under Notifications and the duplicates stop.", fromCustomer: false, age: 128 },

  { ticket: "training", body: "We are opening a branch in Denver in October. Can we book an onboarding session for six new staff?", fromCustomer: true, age: 150 },
  { ticket: "training", body: "Yes. I have sent three slots in the first week of October. Pick whichever suits.", fromCustomer: false, age: 140 },

  { ticket: "mobile", body: "On phones the listing gallery clips the right-hand photo.", fromCustomer: true, age: 120 },
  { ticket: "mobile", body: "Fixed in this morning's release. Please refresh and check.", fromCustomer: false, age: 110 },
  { ticket: "mobile", body: "Looks right now. Thanks.", fromCustomer: true, age: 105 },

  { ticket: "password-reset", body: "Requested a reset three times. Nothing in inbox or spam.", fromCustomer: true, age: 200 },
  { ticket: "password-reset", body: "Your address was on our bounce list from an earlier hard bounce. I have cleared it. Please try again.", fromCustomer: false, age: 199 },
  { ticket: "password-reset", body: "Got it this time.", fromCustomer: true, age: 197 },

  { ticket: "report-totals", body: "The August report says 14,203 events but the dashboard says 14,210 for the same range.", fromCustomer: true, age: 240 },
  { ticket: "report-totals", body: "The report was generated at 23:58 on the 31st and 7 events arrived after that. Regenerating the report gives 14,210.", fromCustomer: false, age: 230 },

  { ticket: "slack", body: "We renamed our Slack workspace and notifications stopped.", fromCustomer: true, age: 260 },
  { ticket: "slack", body: "A rename revokes the old webhook. Reconnect Slack under Integrations and it will work again.", fromCustomer: false, age: 255 },
  { ticket: "slack", body: "Reconnected. Working.", fromCustomer: true, age: 250 },

  { ticket: "print", body: "Printing the pick list drops the Notes column off the right edge.", fromCustomer: true, age: 300 },
  { ticket: "print", body: "Print now uses landscape for tables with more than six columns. Released today.", fromCustomer: false, age: 280 },

  { ticket: "two-factor", body: "Lost my phone with the authenticator app. Locked out of the admin account.", fromCustomer: true, age: 320 },
  { ticket: "two-factor", body: "I have verified your identity against the billing contact and reset 2FA. You will be asked to set it up again at next login.", fromCustomer: false, age: 319.5 },
  { ticket: "two-factor", body: "Verified via callback to the number on file.", fromCustomer: false, internal: true, age: 319.5 },

  { ticket: "currency", body: "Our workspace shows USD but we invoice in SEK.", fromCustomer: true, age: 360 },
  { ticket: "currency", body: "Changed under Settings, Billing, Display currency. Existing amounts are not converted, only formatted.", fromCustomer: false, age: 350 },

  { ticket: "attachment-size", body: "Cannot attach the signed enrolment packs. They are around 14 MB each.", fromCustomer: true, age: 400 },
  { ticket: "attachment-size", body: "The limit is now 25 MB per file for your plan.", fromCustomer: false, age: 390 },

  { ticket: "webhook", body: "Every webhook is delivered twice, about 30 seconds apart.", fromCustomer: true, age: 500 },
  { ticket: "webhook", body: "Your endpoint returns 200 after 12 seconds. Our timeout is 10, so we retry. Respond immediately and process in the background, or we can raise the timeout for your account.", fromCustomer: false, age: 490 },
  { ticket: "webhook", body: "Changed our handler to ack first. Duplicates gone.", fromCustomer: true, age: 470 },

  { ticket: "billing-address", body: "We moved. New address is 41 Cannery Row, Portland, OR 97209.", fromCustomer: true, age: 520 },
  { ticket: "billing-address", body: "Updated. The next invoice will show the new address.", fromCustomer: false, age: 515 },

  { ticket: "old-import", body: "Some customer notes from the old system did not come across in the import.", fromCustomer: true, age: 600 },
  { ticket: "old-import", body: "Notes over 10,000 characters were skipped by the importer. I have re-run it for the 31 affected records.", fromCustomer: false, age: 590 },

  { ticket: "api-key", body: "A contractor had access to the warehouse integration key. Please rotate it.", fromCustomer: true, age: 650 },
  { ticket: "api-key", body: "Rotated. The old key stops working in 24 hours. The new one is in Settings, API.", fromCustomer: false, age: 648 },

  { ticket: "welcome", body: "Where do I edit the text of the welcome email new members get?", fromCustomer: true, age: 700 },
  { ticket: "welcome", body: "Settings, Emails, Welcome. Changes apply to invites sent after you save.", fromCustomer: false, age: 690 },

  { ticket: "rename-workspace", body: "Please rename our workspace from Kestrel Maintenance to Kestrel Aviation Services.", fromCustomer: true, age: 720 },
  { ticket: "rename-workspace", body: "Done.", fromCustomer: false, age: 715 },

  { ticket: "downtime", body: "The portal was down from 08:10 to 08:30 our time. Drivers could not check in.", fromCustomer: true, age: 800 },
  { ticket: "downtime", body: "Confirmed. A database failover took longer than it should have. A post-incident report is on the status page. Apologies for the disruption.", fromCustomer: false, age: 799.5 },

  { ticket: "cancel-trial", body: "We created a second trial workspace by mistake. Please cancel vantage-solar-2.", fromCustomer: true, age: 900 },
  { ticket: "cancel-trial", body: "Cancelled and scheduled for deletion in 30 days in case you need anything from it.", fromCustomer: false, age: 880 },
];

export interface ShapedSeed {
  customers: Array<{ key: string; row: Record<string, unknown> }>;
  tickets: Array<{ key: string; customer: string; row: Record<string, unknown> }>;
  messages: Array<{ ticket: string; row: Record<string, unknown> }>;
}

export function shapeSeed(now: number = Date.now()): ShapedSeed {
  const hoursAgo = (hours: number) => new Date(now - hours * 3_600_000).toISOString();

  const latestMessage = new Map<string, number>();
  for (const m of SEED_MESSAGES) {
    const current = latestMessage.get(m.ticket);
    if (current === undefined || m.age < current) latestMessage.set(m.ticket, m.age);
  }

  return {
    customers: SEED_CUSTOMERS.map((c, index) => ({
      key: c.key,
      row: { name: c.name, email: c.email, company: c.company, createdAt: hoursAgo(2000 - index * 60) },
    })),
    tickets: SEED_TICKETS.map((t) => ({
      key: t.key,
      customer: t.customer,
      row: {
        subject: t.subject,
        status: t.status,
        priority: t.priority,
        firstRespondedAt: t.respondedAgo === null ? null : hoursAgo(t.respondedAgo),
        createdAt: hoursAgo(t.age),
        updatedAt: hoursAgo(latestMessage.get(t.key) ?? t.age),
      },
    })),
    messages: SEED_MESSAGES.map((m) => ({
      ticket: m.ticket,
      row: {
        body: m.body,
        fromCustomer: m.fromCustomer,
        internal: m.internal ?? false,
        createdAt: hoursAgo(m.age),
      },
    })),
  };
}
