import type { LucideIcon } from "lucide-react";
import {
  BookOpen,
  Code2,
  FolderKanban,
  History,
  KanbanSquare,
  LayoutGrid,
  LifeBuoy,
  Users,
} from "lucide-react";
import { FREE_PROJECT_LIMIT, PLANS, TRIAL_DAYS, formatPrice } from "./plans";

// Business-specific copy and settings. The landing page, the nav and footer,
// the /products, /solutions, /resources, /company, and /compare pages, and the
// sitemap all read this file, so the whole site rebrands from here.
//
// Colors are applied as CSS variables on <html> in app/layout.tsx.
//
// "Acme" and every name, quote, and competitor below are placeholders.
// Replace them with your product's real copy before you launch.

/* ----------------------------- types ----------------------------- */

export type ProductFeature = { title: string; body: string };

export type Product = {
  slug: string;
  icon: LucideIcon;
  title: string; // nav label and page subject
  tagline: string; // one line for the nav menu
  headline: string; // section and page heading
  summary: string; // section and page paragraph
  features: ProductFeature[];
  // A capture of this app's own dashboard (scripts/capture-screenshots.mjs).
  screenshot: string;
  screenshotAlt: string;
  // The region of the screenshot the landing page shows, as fractions of
  // its width and height (see ProductShot in components/marketing.tsx).
  crop: { left: number; top: number; width: number; ratio: number };
  // Shown as a section on the landing page. Every product gets its own
  // /products/<slug> page either way.
  onLanding: boolean;
};

export type ContentSection = { title: string; body: string };

export type SitePage = {
  slug: string;
  navLabel: string; // label in nav/footer
  title: string; // page heading
  summary: string;
  sections: ContentSection[];
};

export type Comparison = {
  slug: string;
  navLabel: string;
  competitor: string;
  title: string;
  summary: string;
  rows: { dim: string; acme: string; them: string }[];
};

export type Social = { label: string; href: string; path: string };
export type Quote = { quote: string; name: string; role: string };
export type Plan = {
  name: string;
  tagline: string;
  price: string;
  unit: string;
  cta: string;
  featured: boolean;
  features: string[];
};
export type Faq = { q: string; a: string };
export type ResourceLink = { icon: LucideIcon; title: string; desc: string; href: string };

export type SiteConfig = {
  brand: {
    name: string;
    letter: string; // logo monogram
    domain: string;
    email: string;
    footerBlurb: string;
    copyrightName: string;
    socials: Social[];
  };
  // Accent + surfaces. Applied as CSS vars on <html> in layout.tsx.
  colors: { brand: string; brandSoft: string; paper: string };
  seo: { title: string; description: string };
  hero: {
    headline: string;
    subcopy: string;
    note: string;
    screenshot: string;
    screenshotAlt: string;
  };
  // One customer quote. Leave `quote` empty to hide the section.
  quote: Quote;
  pricing: { headline: string; body: string; plans: Plan[] };
  finalCta: { headline: string; body: string; cta: string };
  faq: { headline: string; items: Faq[] };
  products: Product[];
  resourcesMenu: ResourceLink[];
  solutions: SitePage[];
  resources: SitePage[];
  company: SitePage[];
  comparisons: Comparison[];
};

/* ----------------------------- config ---------------------------- */

export const siteConfig: SiteConfig = {
  brand: {
    name: "Acme",
    letter: "A",
    domain: "acme.example",
    email: "hello@acme.example",
    footerBlurb: "Projects, tasks, and the people doing them, in one shared workspace.",
    copyrightName: "Acme, Inc.",
    socials: [
      {
        label: "X",
        href: "https://x.com/acme",
        path: "M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z",
      },
      {
        label: "GitHub",
        href: "https://github.com/acme",
        path: "M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12",
      },
    ],
  },

  colors: { brand: "#4353ff", brandSoft: "#eef0ff", paper: "#fafafa" },

  seo: {
    title: "Acme: projects and tasks for small teams",
    description:
      "Acme is a shared workspace for your team's projects and tasks. Assign work, set due dates, and see every change as it happens.",
  },

  hero: {
    headline: "Track every project and task your team is working on.",
    subcopy:
      "Acme keeps your projects, tasks, and owners in one shared workspace. A change made by one teammate shows up on everyone's screen within a second.",
    note: `Free for up to ${FREE_PROJECT_LIMIT} projects. No card required.`,
    screenshot: "/screenshots/overview.png",
    screenshotAlt: "The Acme overview: open tasks by due date, project progress, and recently completed work",
  },

  quote: {
    quote:
      "We dropped our Monday status meeting. Everyone opens the overview, sees what is due this week, and gets to work.",
    name: "Priya Raman",
    role: "Engineering lead, Fieldnote",
  },

  pricing: {
    headline: "Pricing",
    body: `Start free. Pro removes the project limit and starts with a ${TRIAL_DAYS}-day free trial.`,
    // Derived from lib/plans.ts, the catalog the Billing tab and the server
    // cap read, so marketing and billing can't disagree on a price.
    plans: PLANS.map((p) => ({
      name: p.name,
      tagline: p.tagline,
      price: formatPrice(p.monthly),
      unit: p.monthly === 0 ? "forever" : "/ month",
      cta: p.cta,
      featured: p.id === "pro",
      features: p.features,
    })),
  },

  finalCta: {
    headline: "Set up your workspace",
    body: "Create an account, name your workspace, and add your first project. Sample projects are one click away if you want to look around first.",
    cta: "Create a free account",
  },

  faq: {
    headline: "Questions",
    items: [
      {
        q: "What counts toward the free plan's limit?",
        a: `Active projects. A free workspace can have ${FREE_PROJECT_LIMIT} at a time. Archived projects don't count, and tasks and members are unlimited on every plan.`,
      },
      {
        q: "How does the Pro trial work?",
        a: `Pro starts with ${TRIAL_DAYS} days free. Checkout asks for a card, and you are charged when the trial ends unless you cancel from Billing first.`,
      },
      {
        q: "Who can see a workspace's projects?",
        a: "Only its members. Each workspace's projects and tasks are stored separately, and people outside it can't read or change them.",
      },
      {
        q: "What can each role do?",
        a: "Owners and admins invite people, change roles, and manage billing. Members create and edit projects and tasks.",
      },
      {
        q: "Can I belong to more than one workspace?",
        a: "Yes. Switch between them from the menu at the top of the sidebar.",
      },
    ],
  },

  products: [
    {
      slug: "boards",
      icon: KanbanSquare,
      title: "Task boards",
      tagline: "A board with four columns for every project.",
      headline: "A board for every project",
      summary:
        "Tasks sit in four columns: To do, In progress, In review, and Done. Give each one a priority, an owner, and a due date, and change its status right from the card.",
      screenshot: "/screenshots/board.png",
      screenshotAlt: "A project board with tasks in To do, In progress, In review, and Done",
      crop: { left: 0.17, top: 0.07, width: 0.82, ratio: 2 },
      onLanding: true,
      features: [
        { title: "Four statuses", body: "To do, In progress, In review, and Done, in that order on every board." },
        { title: "Priorities", body: "Urgent, high, medium, or low. Urgent work sorts to the top of its column." },
        { title: "Owners and due dates", body: "Assign a teammate and a date. Overdue tasks turn red." },
        { title: "Live changes", body: "When a teammate moves a card, it moves on your screen too." },
      ],
    },
    {
      slug: "overview",
      icon: LayoutGrid,
      title: "Overview",
      tagline: "What is due, and how each project is going.",
      headline: "See what is due this week",
      summary:
        "The overview lists open tasks by due date, shows progress for each project, and lists what the team finished recently. Filter to the tasks assigned to you.",
      screenshot: "/screenshots/overview.png",
      screenshotAlt: "The overview with open tasks by due date and project progress",
      crop: { left: 0.195, top: 0.3, width: 0.77, ratio: 16 / 9 },
      onLanding: true,
      features: [
        { title: "Up next", body: "Open tasks across every project, earliest due date first." },
        { title: "Assigned to me", body: "One click narrows the list to your own work." },
        { title: "Project progress", body: "Done and total tasks for each active project." },
        { title: "Weekly count", body: "Tasks completed in the last seven days." },
      ],
    },
    {
      slug: "projects",
      icon: FolderKanban,
      title: "Projects",
      tagline: "Every project with its progress.",
      headline: "Progress for every project",
      summary:
        "Each project shows its open tasks and a progress bar. Archive a finished project to keep the list short, and restore it when you need it again.",
      screenshot: "/screenshots/projects.png",
      screenshotAlt: "The projects list with progress bars and open task counts",
      crop: { left: 0.27, top: 0.1, width: 0.7, ratio: 16 / 7 },
      onLanding: false,
      features: [
        { title: "Progress bars", body: "The share of each project's tasks that are done." },
        { title: "Archive and restore", body: "Archived projects keep their tasks and leave the active list." },
        { title: "Descriptions", body: "A line under each name that says what the project delivers." },
        { title: "Delete with tasks", body: "Deleting a project removes its tasks with it." },
      ],
    },
    {
      slug: "members",
      icon: Users,
      title: "Members and roles",
      tagline: "Invite your team and set who can do what.",
      headline: "Workspaces, members, and roles",
      summary:
        "Invite people by email. Owners and admins manage members and billing, and members work on projects and tasks. A workspace's data is visible only to its members.",
      screenshot: "/screenshots/members.png",
      screenshotAlt: "The members page with four members and two pending invitations",
      crop: { left: 0.3, top: 0.07, width: 0.54, ratio: 16 / 10 },
      onLanding: true,
      features: [
        { title: "Email invites", body: "Invitees get a link that adds them to the workspace." },
        { title: "Three roles", body: "Owner, admin, and member, changeable at any time." },
        { title: "Many workspaces", body: "One account can belong to several workspaces." },
        { title: "Private by default", body: "People outside a workspace can't read its projects or tasks." },
      ],
    },
  ],

  resourcesMenu: [
    { icon: BookOpen, title: "Docs", desc: "Set up and use Acme.", href: "/resources/docs" },
    { icon: History, title: "Changelog", desc: "What changed, week by week.", href: "/resources/changelog" },
    { icon: Code2, title: "API reference", desc: "Read and write your workspace from code.", href: "/resources/api" },
    { icon: LifeBuoy, title: "Contact", desc: "Talk to the team behind Acme.", href: "/company/contact" },
  ],

  solutions: [
    {
      slug: "startups",
      navLabel: "For startups",
      title: "Keep a small team on the same page",
      summary: "One workspace for every project, with the owner and due date of each task in plain view.",
      sections: [
        { title: "Set up in minutes", body: "Name the workspace, invite the team, and add a project." },
        { title: "Free to start", body: `Up to ${FREE_PROJECT_LIMIT} active projects on the free plan.` },
        { title: "Room to grow", body: "Pro removes the project limit when you need more." },
      ],
    },
    {
      slug: "agencies",
      navLabel: "For agencies",
      title: "One workspace per client",
      summary: "Keep each client's projects in a separate workspace and switch between them from the sidebar.",
      sections: [
        { title: "Separate data", body: "A client's workspace is visible only to the people you invite to it." },
        { title: "Invite the client", body: "Add client contacts as members so they can follow the board." },
        { title: "Archive finished work", body: "Archived projects keep their history out of the active list." },
      ],
    },
  ],

  resources: [
    {
      slug: "docs",
      navLabel: "Docs",
      title: "Documentation",
      summary: "How to set up a workspace, invite your team, and run projects in Acme.",
      sections: [
        { title: "Getting started", body: "Create a workspace, invite your team, and add your first project." },
        { title: "Boards", body: "Statuses, priorities, owners, and due dates." },
        { title: "Billing", body: "Plans, the Pro trial, and invoices." },
      ],
    },
    {
      slug: "changelog",
      navLabel: "Changelog",
      title: "Changelog",
      summary: "What we shipped, newest first.",
      sections: [
        { title: "This week", body: "An Up next list on the overview, sorted by due date." },
        { title: "Last week", body: "Archive and restore for projects." },
        { title: "Earlier", body: "Task boards with priorities and due dates." },
      ],
    },
    {
      slug: "api",
      navLabel: "API reference",
      title: "API reference",
      summary: "Read and write your workspace's projects and tasks over HTTP.",
      sections: [
        { title: "Authentication", body: "Requests carry a session token scoped to one workspace." },
        { title: "Projects and tasks", body: "List, create, and update them with the same checks the app uses." },
        { title: "Live updates", body: "Subscribe to changes over a WebSocket." },
      ],
    },
  ],

  company: [
    {
      slug: "about",
      navLabel: "About",
      title: "About Acme",
      summary: "A small team building a fast, focused place to track work.",
      sections: [
        { title: "What we build", body: "Project and task tracking for teams of two to fifty." },
        { title: "How we work", body: "Small team, weekly releases." },
        { title: "Where we are", body: "Remote, across several time zones." },
      ],
    },
    {
      slug: "contact",
      navLabel: "Contact",
      title: "Contact",
      summary: "Reach the Acme team about sales, support, or press.",
      sections: [
        { title: "Sales", body: "hello@acme.example" },
        { title: "Support", body: "support@acme.example, answered within one business day." },
        { title: "Press", body: "press@acme.example" },
      ],
    },
    {
      slug: "privacy",
      navLabel: "Privacy",
      title: "Privacy",
      summary: "How Acme handles your data.",
      sections: [
        { title: "What we collect", body: "Your account details and the content you add to your workspaces." },
        { title: "How we use it", body: "To run Acme for you. We don't sell it." },
        { title: "Your control", body: "Delete your account or a workspace at any time from Settings." },
      ],
    },
    {
      slug: "terms",
      navLabel: "Terms",
      title: "Terms of Service",
      summary: "The rules for using Acme.",
      sections: [
        { title: "Using Acme", body: "Use it for lawful work, and don't abuse the service or other customers." },
        { title: "Your content", body: "You own your data. You grant us only what we need to run the product for you." },
        { title: "Changes and cancellation", body: "Cancel any time. We give notice before any material change to these terms." },
      ],
    },
  ],

  // Made-up competitors, so the template ships no real brand names. Replace
  // them with real comparisons, or delete the entries to drop the pages.
  comparisons: [
    {
      slug: "beacon",
      navLabel: "Acme vs Beacon",
      competitor: "Beacon",
      title: "Acme vs Beacon",
      summary: "Beacon is built for large programs. Acme is built for small teams that want to start the same day.",
      rows: [
        { dim: "Setup", acme: "One form and you're in", them: "Guided onboarding call" },
        { dim: "Live updates", acme: "Every screen, within a second", them: "On refresh" },
        { dim: "Free plan", acme: `${FREE_PROJECT_LIMIT} active projects`, them: "14-day trial only" },
        { dim: "Members", acme: "Unlimited on every plan", them: "Priced per seat" },
      ],
    },
    {
      slug: "orbit",
      navLabel: "Acme vs Orbit",
      competitor: "Orbit",
      title: "Acme vs Orbit",
      summary: "Orbit is a spreadsheet with task features. Acme is a task board with the essentials and nothing else.",
      rows: [
        { dim: "Main view", acme: "Board per project", them: "Grid" },
        { dim: "Priorities", acme: "Built in", them: "Custom column" },
        { dim: "Due-date warnings", acme: "Built in", them: "Formula" },
        { dim: "Members", acme: "Unlimited on every plan", them: "Priced per seat" },
      ],
    },
  ],
};

/* ------------------------- lookups + helpers ---------------------- */
// `@/lib/products` and `@/lib/site` re-export these.

export const PRODUCTS = siteConfig.products;
export function productBySlug(slug: string): Product | undefined {
  return PRODUCTS.find((p) => p.slug === slug);
}

export const SOLUTIONS = siteConfig.solutions;
export const RESOURCES = siteConfig.resources;
export const COMPANY = siteConfig.company;
export const COMPARISONS = siteConfig.comparisons;

export function bySlug<T extends { slug: string }>(
  list: T[],
  slug: string,
): T | undefined {
  return list.find((x) => x.slug === slug);
}
