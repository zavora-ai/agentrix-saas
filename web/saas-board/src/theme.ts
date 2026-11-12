// AG Studio in the Agentrix look: the kit's indigo and coral, its fonts (served by saas-web at
// /static/ui/tokens.css), light and dark.

import { studioTheme } from "ag-studio";

const font = '"DM Sans", "Manrope", system-ui, -apple-system, "Segoe UI", sans-serif';

export const agentrixTheme = studioTheme
  .withParams(
    {
      accentColor: "#38356d",
      backgroundColor: "#ffffff",
      foregroundColor: "#23223a",
      textColor: "#23223a",
      borderColor: "#e3e2ec",
      fontFamily: font,
      browserColorScheme: "light",
      chartPaletteFills1Color: "#38356d",
      chartPaletteFills2Color: "#de4060",
      chartPaletteFills3Color: "#2f55c4",
      chartPaletteFills4Color: "#11734a",
    } as any,
    "light",
  )
  .withParams(
    {
      accentColor: "#a9a6ff",
      backgroundColor: "#17162b",
      foregroundColor: "#ecebf6",
      textColor: "#ecebf6",
      borderColor: "#2c2a57",
      fontFamily: font,
      browserColorScheme: "dark",
      chartPaletteFills1Color: "#a9a6ff",
      chartPaletteFills2Color: "#ff7d97",
      chartPaletteFills3Color: "#7aa2ff",
      chartPaletteFills4Color: "#4ccf91",
    } as any,
    "dark",
  );
