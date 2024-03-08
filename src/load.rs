use std::io::{ prelude::*, BufReader };
use std::fs::File;
use std::collections::HashMap;

pub fn load_env() -> HashMap<String, String> {
  let mut buf_reader = BufReader::new(File::open(".env").unwrap());
  let mut contents = String::new();
  buf_reader.read_to_string(&mut contents).unwrap();

  let mut env: HashMap<String, String> = HashMap::new();

  let lines: Vec<&str> = contents.split('\n').collect();

  for line in lines {
    let parts: Vec<&str> = line.split('=').collect();
    if parts.len() == 1 {
      continue;
    }
    env.insert(parts[0].to_string(), parts[1..].join("="));
  }

  return env;
}

pub fn get_intranet_subdomains() -> HashMap<String, String> {
  let mut buf_reader = BufReader::new(File::open("intranet_subdomains.csv").unwrap());
  let mut contents = String::new();
  buf_reader.read_to_string(&mut contents).unwrap();

  let mut intranet_subdomains: HashMap<String, String> = HashMap::new();

  let lines: Vec<&str> = contents.split('\n').collect();

  for line in lines {
    let parts: Vec<&str> = line.split(',').collect();
    if parts.len() == 1 {
      continue;
    }
    intranet_subdomains.insert(parts[0].to_string(), parts[1].to_string());
  }

  return intranet_subdomains;
}
