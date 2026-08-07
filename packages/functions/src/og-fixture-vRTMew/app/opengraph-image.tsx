import React from "react";
       export default function OG() {
         return {
           __pylonImageResponse: true,
           element: React.createElement("div",
             { style: { display: "flex", width: "100%", height: "100%", fontSize: 40, fontFamily: "Inter" } },
             "Branded"),
           options: { width: 500, height: 300, headers: { "cache-control": "public, max-age=60" } },
         };
       }