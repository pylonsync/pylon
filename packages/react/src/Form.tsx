import React from "react";

// Pylon's progressive-enhancement form. Renders a plain `<form method="post">`
// that works WITHOUT client JS: the browser POSTs to `action` (a route.ts
// path), the handler processes + redirects (303 See Other), and the browser
// follows with a GET (POST-redirect-GET). WITH JS, the runtime's global submit
// handler intercepts (via `data-pylon-form`), `fetch()`es the same endpoint,
// and follows the redirect through client-side navigation — no full reload.
//
// `<form method>` is POST-only without JS (browsers only do GET/POST natively),
// so POST is the default + the no-JS-safe choice. Set `navigate={false}` to
// force a native full-page submit even when JS is available.
export interface FormProps
  extends Omit<React.FormHTMLAttributes<HTMLFormElement>, "method" | "action"> {
  /** The `route.ts` handler path (e.g. "/notes"). */
  action: string;
  /** HTTP method. Default "post" (the only method usable without JS). */
  method?: "post" | "put" | "patch" | "delete";
  /** Opt out of client interception — force a native full-page submit. */
  navigate?: boolean;
  children?: React.ReactNode;
}

export function Form({
  action,
  method = "post",
  navigate = true,
  children,
  ...rest
}: FormProps) {
  return (
    <form
      method={method}
      action={action}
      {...(navigate ? { "data-pylon-form": "" } : {})}
      {...rest}
    >
      {children}
    </form>
  );
}
