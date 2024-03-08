use std::collections::HashMap;

pub fn get_path(url: &str) -> &str {
  url.split('?').next().unwrap()
}

pub fn get_queries(url: &str) -> HashMap<String, String> {
  let query_string = url.split('?').collect::<Vec<&str>>()[1..].join("?");
  let sections: Vec<&str> = query_string.split('&').collect();

  let mut queries: HashMap<String, String> = HashMap::new();

  for section in sections {
    let parts: Vec<&str> = section.split('=').collect();
    if parts.len() == 1 {
      continue;
    }
    queries.insert(parts[0].to_string(), parts[1..].join("=").to_string());
  }

  return queries;
}
