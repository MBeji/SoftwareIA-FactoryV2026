import type { Metadata } from "next";

import { AppBar } from "@/components/app-bar";
import { TooltipProvider } from "@/components/ui/tooltip";

import "./globals.css";

export const metadata: Metadata = {
  title: "OmniForge AI — Mission Control",
  description: "Software factory agentique : orchestration multi-agents, cycle TDD et supervision HITL / HOTL / autonome.",
};

export default function RootLayout({ children }: LayoutProps<"/">) {
  return (
    <html lang="fr" className="h-full antialiased">
      <body className="flex min-h-full flex-col">
        <TooltipProvider delayDuration={250}>
          <AppBar />
          {children}
        </TooltipProvider>
      </body>
    </html>
  );
}
