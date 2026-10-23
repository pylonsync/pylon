// Namespace barrels (`radix-ui`): parsing the entry, and rewriting named
// imports of it into imports of the packages it re-exports.

import { describe, expect, test } from "bun:test";
import { parseNamespaceBarrel, rewriteBarrelImports } from "./ssr-barrel-imports";

const RADIX = `// src/index.ts
"use client";
import * as AlertDialog from "@radix-ui/react-alert-dialog";
import * as Dialog from "@radix-ui/react-dialog";
import * as unstable_PasswordToggleField from "@radix-ui/react-password-toggle-field";
export {
  AlertDialog,
  Dialog,
  unstable_PasswordToggleField
};
//# sourceMappingURL=index.mjs.map
`;

describe("parseNamespaceBarrel", () => {
  test("maps each exported namespace to its package", () => {
    expect(Object.fromEntries(parseNamespaceBarrel(RADIX)!)).toEqual({
      AlertDialog: "@radix-ui/react-alert-dialog",
      Dialog: "@radix-ui/react-dialog",
      unstable_PasswordToggleField: "@radix-ui/react-password-toggle-field",
    });
  });

  test("anything else is not a barrel", () => {
    expect(parseNamespaceBarrel(`export const x = 1;`)).toBeNull();
    expect(parseNamespaceBarrel(`import * as A from "a"; export { A }; console.log(1);`)).toBeNull();
    expect(parseNamespaceBarrel(`import { a } from "a"; export { a };`)).toBeNull();
    expect(parseNamespaceBarrel(`import * as A from "a"; export { A, B };`)).toBeNull();
    expect(parseNamespaceBarrel(``)).toBeNull();
  });
});

describe("rewriteBarrelImports", () => {
  const map = parseNamespaceBarrel(RADIX)!;
  const barrels = (spec: string) => (spec === "radix-ui" ? map : null);
  const same = (_barrel: string, spec: string) => spec;

  test("one namespace import per name, keeping local names", () => {
    expect(
      rewriteBarrelImports(`import { Dialog as SheetPrimitive } from "radix-ui";`, barrels, same),
    ).toBe(`import * as SheetPrimitive from "@radix-ui/react-dialog";`);
    expect(
      rewriteBarrelImports(`import {\n  AlertDialog,\n  Dialog,\n} from 'radix-ui'`, barrels, same),
    ).toBe(`import * as AlertDialog from '@radix-ui/react-alert-dialog'; import * as Dialog from '@radix-ui/react-dialog';`);
  });

  test("type imports, unknown names and other packages stay", () => {
    expect(rewriteBarrelImports(`import type { Dialog } from "radix-ui";`, barrels, same)).toBe(
      `import type { Dialog } from "radix-ui";`,
    );
    expect(
      rewriteBarrelImports(`import { Dialog, type Foo, Missing } from "radix-ui";`, barrels, same),
    ).toBe(
      `import { type Foo, Missing } from "radix-ui"; import * as Dialog from "@radix-ui/react-dialog";`,
    );
    const other = `import { useState } from "react";`;
    expect(rewriteBarrelImports(other, barrels, same)).toBe(other);
  });

  test("the target specifier comes from the resolver", () => {
    expect(
      rewriteBarrelImports(`import { Dialog } from "radix-ui";`, barrels, () => "/abs/react-dialog/index.mjs"),
    ).toBe(`import * as Dialog from "/abs/react-dialog/index.mjs";`);
  });
});
