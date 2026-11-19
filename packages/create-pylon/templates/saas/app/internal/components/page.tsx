import React from "react";
import { type Metadata } from "@pylonsync/react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import { Example, Showcase } from "../showcase";

export const metadata: Metadata = {
  title: "Components",
  robots: "noindex, nofollow",
};

// `/internal/components` shows every reusable component in `components/`.
// Look here before you write UI. When you add a component, add a `Showcase`
// for it in the same commit.

const PLANS = [
  { value: "free", label: "Free" },
  { value: "pro", label: "Pro" },
  { value: "team", label: "Team" },
  { value: "enterprise", label: "Enterprise", disabled: true },
];

const OWNERS = [
  { value: "", label: "Unassigned" },
  { value: "ada", label: "Ada" },
  { value: "grace", label: "Grace" },
];

export default function ComponentsPage() {
  return (
    <div className="flex flex-col gap-10">
      <header className="flex flex-col gap-2">
        <h1 className="text-3xl font-semibold tracking-tight">Components</h1>
        <p className="text-sm text-muted-foreground">
          The reusable components in <code>components/</code>. A page composes
          these components. Behavior comes from <code>@ui-sdk/react</code>.
          Styles come from the design tokens.
        </p>
      </header>

      <Showcase
        id="button"
        title="Button"
        source="components/ui/button.tsx"
        description="Use a button to start an action. Use the render prop to show a link as a button."
      >
        <Example label="Variants">
          <Button>Default</Button>
          <Button variant="secondary">Secondary</Button>
          <Button variant="outline">Outline</Button>
          <Button variant="ghost">Ghost</Button>
          <Button variant="link">Link</Button>
          <Button variant="destructive">Destructive</Button>
        </Example>
        <Example label="Sizes">
          <Button size="sm">Small</Button>
          <Button>Default</Button>
          <Button size="lg">Large</Button>
        </Example>
        <Example label="States">
          <Button disabled>Disabled</Button>
          <Button
            onClick={() => new Promise((resolve) => setTimeout(resolve, 1500))}
            pendingAnnouncement="Saving"
          >
            Pending for 1.5 s
          </Button>
          <Button variant="outline" render={<a href="/internal/design" />}>
            Link as button
          </Button>
        </Example>
      </Showcase>

      <Showcase
        id="badge"
        title="Badge"
        source="components/ui/badge.tsx"
        description="Use a badge to show a short status."
      >
        <Example label="Variants">
          <Badge>Default</Badge>
          <Badge variant="secondary">Secondary</Badge>
          <Badge variant="outline">Outline</Badge>
          <Badge variant="destructive">Destructive</Badge>
        </Example>
      </Showcase>

      <Showcase
        id="text-fields"
        title="Input, Label and Textarea"
        source="components/ui/input.tsx, label.tsx, textarea.tsx"
        description="Use these components for text entry. Give each control a label."
      >
        <Example label="Input">
          <div className="flex w-full flex-col gap-1.5">
            <Label htmlFor="showcase-email">Email</Label>
            <Input id="showcase-email" type="email" placeholder="you@example.com" />
          </div>
        </Example>
        <Example label="Disabled input">
          <Input aria-label="Disabled input" disabled placeholder="Disabled" />
        </Example>
        <Example label="Textarea">
          <div className="flex w-full flex-col gap-1.5">
            <Label htmlFor="showcase-notes">Notes</Label>
            <Textarea id="showcase-notes" placeholder="Write a note" />
          </div>
        </Example>
      </Showcase>

      <Showcase
        id="select"
        title="Select"
        source="components/ui/select.tsx"
        description="Use a select to pick one value from a list of options."
      >
        <Example label="With a placeholder">
          <div className="flex w-full flex-col gap-1.5">
            <Label htmlFor="showcase-plan">Plan</Label>
            <Select id="showcase-plan" items={PLANS} placeholder="Select a plan" />
          </div>
        </Example>
        <Example label="With a default value">
          <Select aria-label="Plan" items={PLANS} defaultValue="pro" />
        </Example>
        <Example label="With an option for no value">
          <Select aria-label="Owner" items={OWNERS} defaultValue="" />
        </Example>
        <Example label="Disabled">
          <Select aria-label="Plan" items={PLANS} defaultValue="free" disabled />
        </Example>
      </Showcase>

      <Showcase
        id="dialog"
        title="Dialog"
        source="components/ui/dialog.tsx"
        description="Use a dialog for a task that needs the full attention of the user."
      >
        <Example label="Default">
          <Dialog>
            <DialogTrigger render={<Button variant="outline" />}>
              Open the dialog
            </DialogTrigger>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>Rename the item</DialogTitle>
                <DialogDescription>
                  The new name shows in every list.
                </DialogDescription>
              </DialogHeader>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="showcase-name">Name</Label>
                <Input id="showcase-name" defaultValue="First item" />
              </div>
              <DialogFooter>
                <DialogClose render={<Button variant="outline" />}>
                  Cancel
                </DialogClose>
                <DialogClose render={<Button />}>Save</DialogClose>
              </DialogFooter>
            </DialogContent>
          </Dialog>
        </Example>
      </Showcase>

      <Showcase
        id="sheet"
        title="Sheet"
        source="components/ui/sheet.tsx"
        description="Use a sheet to show navigation on a small screen."
      >
        <Example label="Default">
          <Sheet>
            <SheetTrigger render={<Button variant="outline" />}>
              Open the sheet
            </SheetTrigger>
            <SheetContent className="flex-col gap-2 p-4">
              <SheetTitle className="text-base font-semibold">
                Navigation
              </SheetTitle>
              <SheetDescription className="text-sm text-muted-foreground">
                Links to the sections of the app.
              </SheetDescription>
              <SheetClose render={<Button variant="outline" size="sm" />}>
                Close
              </SheetClose>
            </SheetContent>
          </Sheet>
        </Example>
      </Showcase>

      <Showcase
        id="card"
        title="Card"
        source="components/ui/card.tsx"
        description="Use a card to group related content."
      >
        <Card className="w-full">
          <CardHeader>
            <CardTitle>Team plan</CardTitle>
            <CardDescription>10 seats. Billed each month.</CardDescription>
          </CardHeader>
          <CardContent className="text-sm">
            The plan renews on the first day of each month.
          </CardContent>
          <CardFooter>
            <Button size="sm">Change the plan</Button>
          </CardFooter>
        </Card>
      </Showcase>

      <Showcase
        id="table"
        title="Table"
        source="components/ui/table.tsx"
        description="Use a table to show rows of records."
      >
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Status</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow>
              <TableCell>First item</TableCell>
              <TableCell>
                <Badge variant="secondary">Open</Badge>
              </TableCell>
            </TableRow>
            <TableRow>
              <TableCell>Second item</TableCell>
              <TableCell>
                <Badge variant="outline">Closed</Badge>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </Showcase>
    </div>
  );
}
