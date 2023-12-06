use borsh::{ BorshDeserialize, BorshSerialize };
use solana_program::{
    account_info::{ next_account_info, AccountInfo },
    entrypoint,
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
    keccak::hash,
    system_program::ID as system_program_address,
    borsh0_10::try_from_slice_unchecked,
    sysvar::{
        Sysvar,
        clock
    },
    program::invoke_signed,
    system_instruction::create_account,
    rent
};
use thiserror::Error;

#[derive(BorshDeserialize, BorshSerialize, Debug)]
struct CreateVotingInstruction {
    starts_at: u32,
    ends_at: u32,
    title: String
}

#[derive(BorshDeserialize, BorshSerialize, Debug)]
struct VoteInstruction {
    vote: bool,
    vote_title: String
}

#[derive(BorshDeserialize, BorshSerialize, Debug)]
struct UpdateVoteInstruction {
    vote: bool,
    vote_title: String
}

#[derive(BorshDeserialize, BorshSerialize, Debug)]
struct VoteMainAccount {
    discriminator: [u8; 8],
    creator: Pubkey,
    starts_at: u32,
    ends_at: u32,
    title: String
}

#[derive(BorshDeserialize, BorshSerialize, Debug)]
struct UserVotingAccount {
    discriminator: [u8; 8],
    last_time_voted: u32,
    vote_status: bool,
    voted_to: String
}

#[derive(Error, Debug)]
enum Errors {
    #[error("Starting time < Current time")]
    InvalidStartingTime,
    #[error("Ending time < Starting time")]
    InvalidEndingTime,
    #[error("Max voting time exceeded.")]
    MaxVotingTimeExceeded,
    #[error("Invalid system program account.")]
    InvalidSystemProgram,
    #[error("Invalid PDA seeds.")]
    InvalidPdaAddress,
    #[error("User must be signer.")]
    UserSigningNeeded,
    #[error("User's account must be writable.")]
    UsersAccountMustBeMutable,
    #[error("PDA's account must be writable.")]
    PDAsAccountMustBeMutable,
    #[error("Title length >= 10")]
    TitleInvalidLength,
    #[error("Invalid account owner.")]
    InvalidAccountOwner,
    #[error("Voting has not started yet.")]
    VotingNotStarted,
    #[error("Voting has been ended.")]
    VotingEnded
}

entrypoint!(process_instruction);

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    _instruction_data: &[u8]
) -> ProgramResult {
    let accounts = &mut accounts.iter();
    
    // Constants
    const MAX_VOTING_TIME: u32 = 1_209_600; // 2 weeks
    
    // Discriminators
    //  Instructions
    let create_voting_ix: &[u8] = &hash(b"instruction:create_voting").0[..8];
    let vote_ix: &[u8] = &hash(b"instruction:vote").0[..8];
    let update_vote_ix: &[u8] = &hash(b"instruction:update_vote").0[..8];
    //  Accounts
    let vote_acc: &[u8] = &hash(b"account:vote").0[..8];
    let user_voting_acc: &[u8] = &hash(b"account:user_voting").0[..8];

    // Handle Instruction Indentifier
    let ix_dis = _instruction_data.get(..=7).unwrap();
    if ix_dis == create_voting_ix {
        let user = next_account_info(accounts)?;
        let pda = next_account_info(accounts)?;
        let system_program = next_account_info(accounts)?;

        let current_time = clock::Clock::get().unwrap().unix_timestamp as u32;

        let data = _instruction_data.get(8..).unwrap();
        let ix_data = try_from_slice_unchecked::<CreateVotingInstruction>(data)?;

        if user.is_signer == false {
            return Err(ProgramError::Custom(Errors::UserSigningNeeded as u32));
        };

        if user.is_writable == false {
            return Err(ProgramError::Custom(Errors::UsersAccountMustBeMutable as u32));
        };

        if pda.is_writable == false {
            return Err(ProgramError::Custom(Errors::PDAsAccountMustBeMutable as u32));
        };

        if *system_program.key != system_program_address {
            return Err(ProgramError::Custom(Errors::InvalidSystemProgram as u32));
        };

        let (pda_addr, pda_bump) = Pubkey::find_program_address(
            &[
                b"voting_account".as_ref(),
                ix_data.title.as_bytes().as_ref()
            ],
            program_id
        );
        if pda_addr != *pda.key {
            return Err(ProgramError::Custom(Errors::InvalidPdaAddress as u32));
        };

        if ix_data.starts_at < current_time {
            return Err(ProgramError::Custom(Errors::InvalidStartingTime as u32));
        };

        if ix_data.ends_at <= ix_data.starts_at {
            return Err(ProgramError::Custom(Errors::InvalidEndingTime as u32));
        };

        if ix_data.title.len() < 10 {
            return Err(ProgramError::Custom(Errors::TitleInvalidLength as u32));
        };

        if (ix_data.ends_at - ix_data.starts_at) > MAX_VOTING_TIME.into() {
            return Err(ProgramError::Custom(Errors::MaxVotingTimeExceeded as u32));
        };

        let space: usize = 8 + 32 + 4 + 4 + (4 + 50);
        let rent_exempt = rent::Rent::get().unwrap().minimum_balance(space);
        invoke_signed(
            &create_account(
                user.key,
                &pda_addr,
                rent_exempt,
                space as u64,
                program_id
            ),
            &[
                user.clone(),
                pda.clone(),
                system_program.clone()
            ],
            &[
                &[
                    b"new_voting_account".as_ref(),
                    ix_data.title.as_bytes().as_ref(),
                    user.key.as_ref(),
                    &[ pda_bump ]
                ]
            ]
        )?;

        let _data = pda.data.borrow();
        let mut vote_account = try_from_slice_unchecked::<VoteMainAccount>(_data.get(..).unwrap())?;
        vote_account.discriminator = vote_acc.try_into().unwrap();
        vote_account.creator = *(user.key);
        vote_account.starts_at = ix_data.starts_at;
        vote_account.ends_at = ix_data.ends_at;
        vote_account.title = ix_data.title;
        vote_account.serialize(&mut &mut pda.data.borrow_mut()[..])?;

        msg!("New voting account has been created.");
    } else if ix_dis == vote_ix {
        let user = next_account_info(accounts)?;
        let voting_account = next_account_info(accounts)?;
        let user_vote_account = next_account_info(accounts)?;
        let system_program = next_account_info(accounts)?;

        if user.is_signer == false {
            return Err(ProgramError::Custom(Errors::UserSigningNeeded as u32));
        };

        if user.is_writable == false {
            return Err(ProgramError::Custom(Errors::UsersAccountMustBeMutable as u32));
        };

        if user_vote_account.is_writable == false {
            return Err(ProgramError::Custom(Errors::PDAsAccountMustBeMutable as u32));
        };

        if *system_program.key != system_program_address {
            return Err(ProgramError::Custom(Errors::InvalidSystemProgram as u32));
        };

        let data = _instruction_data.get(8..).unwrap();
        let ix_data = try_from_slice_unchecked::<VoteInstruction>(data)?;

        let (vote_pda_address, _) = Pubkey::find_program_address(
            &[
                b"vote_account".as_ref(),
                ix_data.vote_title.as_bytes().as_ref()
            ],
            program_id
        );
        if *voting_account.key != vote_pda_address {
            return Err(ProgramError::Custom(Errors::InvalidPdaAddress as u32));
        };

        if voting_account.owner != program_id {
            return Err(ProgramError::Custom(Errors::InvalidAccountOwner as u32));
        };

        let data_2 = voting_account.data.borrow();
        if data_2.get(..=7).unwrap() != vote_acc {
            return Err(ProgramError::InvalidAccountData);
        };

        let current_time = clock::Clock::get().unwrap().unix_timestamp as u32;
        let voting_account_data = try_from_slice_unchecked::<VoteMainAccount>(data_2.get(..).unwrap())?;

        if voting_account_data.starts_at > current_time {
            return Err(ProgramError::Custom(Errors::VotingNotStarted as u32));
        };

        if voting_account_data.ends_at < current_time {
            return Err(ProgramError::Custom(Errors::VotingEnded as u32));
        };

        let (user_pda_addr, user_pda_bump) = Pubkey::find_program_address(
            &[
                b"user_vote".as_ref(),
                voting_account_data.title.as_bytes().as_ref(),
                user.key.as_ref()
            ],
            program_id
        );
        if user_pda_addr != *user_vote_account.key {
            return Err(ProgramError::Custom(Errors::InvalidPdaAddress as u32));
        };

        let space: usize = 8 + 4 + 1 + (4 + 50);
        let rent_exempt = rent::Rent::get().unwrap().minimum_balance(space);
        invoke_signed(
            &create_account(
                user.key,
                user_vote_account.key,
                rent_exempt,
                space as u64,
                program_id
            ),
            &[
                user.clone(),
                user_vote_account.clone(),
                system_program.clone()
            ],
            &[
                &[
                    b"user_vote".as_ref(),
                    voting_account_data.title.as_bytes().as_ref(),
                    user.key.as_ref(),
                    &[ user_pda_bump ]
                ]
            ]
        )?;

        let data_3 = user_vote_account.data.borrow();
        let mut user_account = try_from_slice_unchecked::<UserVotingAccount>(data_3.get(..).unwrap())?;
        user_account.discriminator = user_voting_acc.try_into().unwrap();
        user_account.last_time_voted = current_time;
        user_account.vote_status = ix_data.vote;
        user_account.voted_to = ix_data.vote_title;
        user_account.serialize(&mut &mut user_vote_account.data.borrow_mut()[..])?;

        msg!("Voted successfully.");
        msg!("Voted to - {}", user_account.voted_to);
        msg!("Vote status - {}", user_account.vote_status);
    } else if ix_dis == update_vote_ix {
        let user = next_account_info(accounts)?;
        let voting_account = next_account_info(accounts)?;
        let user_vote_account = next_account_info(accounts)?;

        if user.is_signer == false {
            return Err(ProgramError::Custom(Errors::UserSigningNeeded as u32));
        };

        if voting_account.owner != program_id {
            return Err(ProgramError::Custom(Errors::InvalidAccountOwner as u32));
        };

        if user_vote_account.owner != program_id {
            return Err(ProgramError::Custom(Errors::InvalidAccountOwner as u32));
        };

        if user_vote_account.is_writable == false {
            return Err(ProgramError::Custom(Errors::PDAsAccountMustBeMutable as u32));
        };

        let data = _instruction_data.get(8..).unwrap();
        let ix_data = try_from_slice_unchecked::<UpdateVoteInstruction>(data)?;
        if ix_data.vote_title.len() < 10 {
            return Err(ProgramError::Custom(Errors::TitleInvalidLength as u32));
        };

        let (voting_pda_addr, _) = Pubkey::find_program_address(
            &[
                b"vote_account".as_ref(),
                ix_data.vote_title.as_bytes().as_ref()
            ],
            program_id
        );
        if *voting_account.key != voting_pda_addr {
            return Err(ProgramError::Custom(Errors::InvalidPdaAddress as u32));
        };

        let (user_vote_pda_addr, _) = Pubkey::find_program_address(
            &[
                b"user_vote".as_ref(),
                ix_data.vote_title.as_bytes().as_ref(),
                user.key.as_ref()
            ],
            program_id
        );
        if *user_vote_account.key != user_vote_pda_addr {
            return Err(ProgramError::Custom(Errors::InvalidPdaAddress as u32));
        };

        let data = &voting_account.data.borrow()[..];
        if data.get(..8).unwrap() != vote_acc {
            return Err(ProgramError::InvalidAccountData);
        };

        let current_time = clock::Clock::get().unwrap().unix_timestamp as  u32;
        let voting_account_data = try_from_slice_unchecked::<VoteMainAccount>(&data)?;

        if voting_account_data.starts_at > current_time {
            return Err(ProgramError::Custom(Errors::VotingNotStarted as u32));
        };

        if voting_account_data.ends_at <= current_time {
            return Err(ProgramError::Custom(Errors::VotingEnded as u32));
        };

        let data_2 = &user_vote_account.data.borrow()[..];
        if data_2.get(..8).unwrap() != user_voting_acc {
            return Err(ProgramError::InvalidAccountData);
        };

        let mut user_vote_account_data = try_from_slice_unchecked::<UserVotingAccount>(&data_2)?;
        user_vote_account_data.vote_status = ix_data.vote;
        user_vote_account_data.last_time_voted = current_time;
        user_vote_account_data.serialize(&mut &mut user_vote_account.data.borrow_mut()[..])?;

        msg!("Vote updated.");
    } else {
        return Err(ProgramError::InvalidInstructionData);
    };

    Ok(())
}
