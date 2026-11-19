// The sections of the internal pages. The sidebar and the pages read this
// list, so add an entry here when you add a section to a page.

export interface InternalSection {
  /** The id of the section element. The sidebar links to it. */
  id: string;
  title: string;
}

export const DESIGN_SECTIONS: InternalSection[] = [
  { id: "color", title: "Color" },
  { id: "type", title: "Type" },
  { id: "radius", title: "Radius" },
  { id: "spacing", title: "Spacing" },
  { id: "shadow", title: "Shadow" },
];

export const COMPONENT_SECTIONS: InternalSection[] = [
  { id: "button", title: "Button" },
  { id: "badge", title: "Badge" },
  { id: "text-fields", title: "Input, Label, Textarea" },
  { id: "select", title: "Select" },
  { id: "dialog", title: "Dialog" },
  { id: "card", title: "Card" },
  { id: "table", title: "Table" },
];
