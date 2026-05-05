import { ErpApp } from "../../client/ErpApp";

// Pure-client demo: the existing client component owns all of the
// rendering. Wrapping it in a server component (default) keeps the
// Next.js app shape consistent across examples while leaving the
// example's UI untouched.
export default function Page() {
  return <ErpApp />;
}
