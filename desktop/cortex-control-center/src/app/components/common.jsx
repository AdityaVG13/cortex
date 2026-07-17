import React from "react";
function EmptyItem({ text }) {
  return React.createElement("li", { className: "empty" }, text);
}
export { EmptyItem };
