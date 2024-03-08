pub fn to_binary(num: impl std::fmt::Binary, pad: bool) -> String {
  let mut unpadded = format!("{:b}", num);
  if unpadded.len() < 6 && pad {
    unpadded = "0".repeat(6 - unpadded.len()) + &unpadded;
  }
  unpadded
}

//input MUST be len 8 (or less) string of 0 or 1
//should be Result<> but can't be bothered right now
pub fn binary_to_u8(binary_chars: &str) -> u8 {
  let mut bc_iter = binary_chars.chars();
  let mut total: u8 = 0;
  let mut start = 1;
  if binary_chars.len() < 8 {
    start = 8 - binary_chars.len() - 1;
  }
  for i in start..=8 {
    let c = bc_iter.next().unwrap();
    if c == '1' {
      total += 2_u8.pow((8 - i).try_into().unwrap());
    }
  }
  return total;
}

const B64_CHARS: [char; 64] = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', 'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '-', '_'];

//seems to work but might have extra 0s at end
pub fn b64_url_to_u8_vec(b64: &String) -> Result<Vec<u8>, ()> {
  let mut u8_vec = Vec::new();

  let mut binary = String::new();

  //b64 means 6 bits per char
  //convert into binary
  for c in b64.chars() {
    if c != '=' {
      if let Some(b64_pos) = B64_CHARS.iter().position(|&bc| c == bc) {
        binary = binary + &to_binary(b64_pos, true);
      } else {
        //invalid b64, unrecognized character
        return Err(());
      }
    }
  }

  let mod_b = binary.len() % 8;
  if binary.len() % 8 != 0 {
    binary = binary + &"0".repeat(8 - mod_b);
  }

  //turn each 8 bytes of binary into a u8, add to vec
  //(if last one is not 8, pad with zeroes)
  for i in 0..(binary.len() / 8) {
    u8_vec.push(binary_to_u8(binary.get((i * 8)..(i * 8 + 8)).unwrap()));
  }

  return Ok(u8_vec);
}
