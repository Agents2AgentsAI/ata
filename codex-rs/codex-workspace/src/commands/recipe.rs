use crate::error::WorkspaceError;
use crate::recipes;

/// Print a recipe by name. Returns the recipe text.
pub fn run(operation: &str) -> Result<String, WorkspaceError> {
    if operation == "list" {
        let names = recipes::list_recipes();
        let mut out = "Available recipes:\n".to_string();
        for name in names {
            out.push_str(&format!("  {name}\n"));
        }
        return Ok(out);
    }

    recipes::get_recipe(operation)
        .map(str::to_string)
        .ok_or_else(|| WorkspaceError::UnknownRecipe(operation.to_string()))
}
