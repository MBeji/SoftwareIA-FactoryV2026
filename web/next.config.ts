import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Self-contained server bundle, for the Docker image only (see Dockerfile). Vercel and
  // `next start` use the default output.
  output: process.env.NEXT_OUTPUT === "standalone" ? "standalone" : undefined,
  cacheComponents: true,
  partialPrefetching: true,
  turbopack: {
    rules: {
      "*.css": {
        loaders: ["@tailwindcss/turbopack"],
        as: "*.css",
      },
    },
  },
};

export default nextConfig;
