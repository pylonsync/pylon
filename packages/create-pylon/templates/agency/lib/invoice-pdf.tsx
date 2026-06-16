import React from "react";
import { Document, Page, View, Text, StyleSheet, pdf } from "@react-pdf/renderer";
import { money, type InvoiceLineItem } from "./agency";

// Invoice → PDF, with @react-pdf/renderer (the React-to-PDF renderer — note
// `react-pdf` proper is a *viewer* for existing PDFs; this is the generator).
//
// This module is imported DYNAMICALLY from the dashboard's "Download PDF" click
// handler (`await import("@/lib/invoice-pdf")`), never during render — so the
// ~1MB renderer + this document never touch SSR or the initial client bundle;
// it loads only when the owner actually exports a bill. The PDF mirrors the
// on-screen invoice view so what you see is what you download.

export interface InvoicePdfData {
  number: string;
  status: string;
  issuedAt?: string | null;
  dueAt?: string | null;
  notes?: string | null;
  projectTitle?: string | null;
  items: InvoiceLineItem[];
  amountCents: number;
  billTo: { name: string; company?: string | null; email?: string | null };
  studio: {
    name: string;
    addressLines: string[];
    paymentTerms: string;
    footerNote: string;
    brandColor: string;
  };
}

// Build (and return) the invoice PDF as a Blob the caller can download.
export async function buildInvoiceBlob(data: InvoicePdfData): Promise<Blob> {
  return await pdf(<InvoiceDocument data={data} />).toBlob();
}

const STATUS_COLOR: Record<string, string> = {
  draft: "#71717a",
  sent: "#2563eb",
  paid: "#16a34a",
  overdue: "#dc2626",
};

function InvoiceDocument({ data }: { data: InvoicePdfData }) {
  const { studio, billTo } = data;
  const s = makeStyles(studio.brandColor);
  // A bill with no explicit line items still prints one line for its total.
  const items =
    data.items.length > 0
      ? data.items
      : [{ description: "Services", quantity: 1, unitCents: data.amountCents }];

  return (
    <Document title={`Invoice ${data.number}`} author={studio.name}>
      <Page size="A4" style={s.page}>
        {/* Header */}
        <View style={s.header}>
          <View>
            <Text style={s.studioName}>{studio.name}</Text>
            {studio.addressLines.map((l, i) => (
              <Text key={i} style={s.muted}>
                {l}
              </Text>
            ))}
          </View>
          <View style={s.headerRight}>
            <Text style={s.invoiceLabel}>INVOICE</Text>
            <Text style={s.invoiceNumber}>{data.number}</Text>
            <Text style={[s.status, { color: STATUS_COLOR[data.status] ?? "#71717a" }]}>
              {data.status.toUpperCase()}
            </Text>
          </View>
        </View>

        <View style={s.rule} />

        {/* Bill-to + meta */}
        <View style={s.cols}>
          <View style={s.col}>
            <Text style={s.sectionLabel}>BILL TO</Text>
            <Text style={s.strong}>{billTo.name}</Text>
            {billTo.company ? <Text style={s.muted}>{billTo.company}</Text> : null}
            {billTo.email ? <Text style={s.muted}>{billTo.email}</Text> : null}
          </View>
          <View style={s.colRight}>
            {data.projectTitle ? <Meta s={s} label="Project" value={data.projectTitle} /> : null}
            {data.issuedAt ? <Meta s={s} label="Issued" value={data.issuedAt} /> : null}
            {data.dueAt ? <Meta s={s} label="Due" value={data.dueAt} /> : null}
            <Meta s={s} label="Terms" value={studio.paymentTerms} />
          </View>
        </View>

        {/* Line items */}
        <View style={s.table}>
          <View style={s.tableHead}>
            <Text style={[s.cell, s.cDesc, s.headCell]}>Description</Text>
            <Text style={[s.cell, s.cQty, s.headCell]}>Qty</Text>
            <Text style={[s.cell, s.cUnit, s.headCell]}>Unit</Text>
            <Text style={[s.cell, s.cAmt, s.headCell]}>Amount</Text>
          </View>
          {items.map((it, i) => (
            <View key={i} style={s.tableRow}>
              <Text style={[s.cell, s.cDesc]}>{it.description}</Text>
              <Text style={[s.cell, s.cQty]}>{it.quantity}</Text>
              <Text style={[s.cell, s.cUnit]}>{money(it.unitCents)}</Text>
              <Text style={[s.cell, s.cAmt]}>{money(Math.round(it.quantity * it.unitCents))}</Text>
            </View>
          ))}
          <View style={s.totalRow}>
            <Text style={s.totalLabel}>Total due</Text>
            <Text style={s.totalValue}>{money(data.amountCents)}</Text>
          </View>
        </View>

        {data.notes ? (
          <View style={s.notes}>
            <Text style={s.sectionLabel}>NOTES</Text>
            <Text style={s.muted}>{data.notes}</Text>
          </View>
        ) : null}

        <Text style={s.footer} fixed>
          {studio.footerNote}
        </Text>
      </Page>
    </Document>
  );
}

function Meta({ s, label, value }: { s: ReturnType<typeof makeStyles>; label: string; value: string }) {
  return (
    <View style={s.metaRow}>
      <Text style={s.metaLabel}>{label}</Text>
      <Text style={s.metaValue}>{value}</Text>
    </View>
  );
}

function makeStyles(brand: string) {
  return StyleSheet.create({
    page: { padding: 48, fontSize: 10, color: "#27272a", fontFamily: "Helvetica", lineHeight: 1.4 },
    header: { flexDirection: "row", justifyContent: "space-between", alignItems: "flex-start" },
    studioName: { fontSize: 18, fontFamily: "Helvetica-Bold", color: "#18181b", marginBottom: 4 },
    headerRight: { alignItems: "flex-end" },
    invoiceLabel: { fontSize: 11, letterSpacing: 2, color: brand, fontFamily: "Helvetica-Bold" },
    invoiceNumber: { fontSize: 13, marginTop: 2, fontFamily: "Helvetica-Bold", color: "#18181b" },
    status: { fontSize: 9, marginTop: 4, fontFamily: "Helvetica-Bold", letterSpacing: 1 },
    muted: { color: "#71717a" },
    rule: { borderBottomWidth: 1, borderBottomColor: "#e4e4e7", marginVertical: 18 },
    cols: { flexDirection: "row", justifyContent: "space-between", marginBottom: 24 },
    col: { width: "55%" },
    colRight: { width: "40%" },
    sectionLabel: { fontSize: 8, letterSpacing: 1.5, color: "#a1a1aa", fontFamily: "Helvetica-Bold", marginBottom: 5 },
    strong: { fontFamily: "Helvetica-Bold", color: "#18181b", fontSize: 11 },
    metaRow: { flexDirection: "row", justifyContent: "space-between", marginBottom: 3 },
    metaLabel: { color: "#a1a1aa" },
    metaValue: { color: "#27272a", fontFamily: "Helvetica-Bold" },
    table: { marginTop: 4 },
    tableHead: { flexDirection: "row", borderBottomWidth: 1, borderBottomColor: "#27272a", paddingBottom: 6 },
    headCell: { fontFamily: "Helvetica-Bold", color: "#18181b", fontSize: 9, letterSpacing: 0.5 },
    tableRow: { flexDirection: "row", borderBottomWidth: 1, borderBottomColor: "#f4f4f5", paddingVertical: 8 },
    cell: { fontSize: 10 },
    cDesc: { width: "52%", paddingRight: 8 },
    cQty: { width: "12%", textAlign: "right" },
    cUnit: { width: "18%", textAlign: "right" },
    cAmt: { width: "18%", textAlign: "right" },
    totalRow: { flexDirection: "row", justifyContent: "flex-end", marginTop: 12, paddingTop: 4 },
    totalLabel: { fontSize: 11, fontFamily: "Helvetica-Bold", color: "#18181b", marginRight: 16 },
    totalValue: { fontSize: 14, fontFamily: "Helvetica-Bold", color: brand, minWidth: 90, textAlign: "right" },
    notes: { marginTop: 28 },
    footer: { position: "absolute", bottom: 36, left: 48, right: 48, fontSize: 9, color: "#a1a1aa", textAlign: "center" },
  });
}
