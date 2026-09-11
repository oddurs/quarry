import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

export interface TerminalTheme {
  id: string;
  label: string;
  wide: string;
  narrow: string;
}

export function TerminalTabs({ themes }: { themes: TerminalTheme[] }) {
  return (
    <Tabs defaultValue={themes[0]?.id}>
      <TabsList>
        {themes.map((theme) => (
          <TabsTrigger key={theme.id} value={theme.id}>
            {theme.label}
          </TabsTrigger>
        ))}
      </TabsList>

      {themes.map((theme) => (
        <TabsContent key={theme.id} value={theme.id}>
          <div
            className="tui-frame bg-card rounded-lg border p-4 max-sm:hidden"
            dangerouslySetInnerHTML={{ __html: theme.wide }}
          />
          <div
            className="tui-frame tui-narrow bg-card rounded-lg border p-3 sm:hidden"
            dangerouslySetInnerHTML={{ __html: theme.narrow }}
          />
        </TabsContent>
      ))}
    </Tabs>
  );
}
