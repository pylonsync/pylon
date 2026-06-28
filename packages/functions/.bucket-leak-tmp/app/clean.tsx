export const cache = "auth-bucketed";
       export const revalidate = 60;
       export default function Page({ session }: any) {
         return <div>{session.exists ? "in" : "out"}</div>;
       }